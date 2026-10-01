//! Q2 rerelease configstrings, `svc_locprint`, and `cg_screen.cpp` story draws.
//!
//! Sync port of donor `src/app/bootstrap/rerelease-presentation.ts`
//! (GPL-2.0-or-later).
//!
//! Holds client configstrings and interpolation only. Source game callbacks
//! own story changes and application actions. The donor awaits asset-provider
//! promises; this port resolves the same bytes through the synchronous
//! [`RereleasePresentationProvider`], so `prepare` and the image refresh run
//! without suspension.

pub mod fog;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_client::materials::sky::SKY_FACE_SUFFIXES;
use qa_client::render::scene::q2_sky::Q2SkyView;
use qa_client::render::types::{Q2Fog, RendererImage};
use qa_client::text::draw2d::Draw2D;
use qa_client::text::layout::{ColorCodes, SeatTextPresentation, TextAlign, TextLayoutOptionsWithoutFont};
use qa_client::text::localization::{LocLoadTier, LocalizationProfile};
use qa_client::ui::settings::language::{read_seat_language, write_seat_language};
use qa_client::ui::settings::services::{LanguageChoice, LocalizationView, NativeLanguageSettings};
use qa_client::ui::settings::{SettingBinding, SettingBindingKind, SettingCvars};
use qa_client::ClientError;
use qa_content::contract::{ContentId, GameFamily};
use qa_content::q2::base::player::types::{Q2PlayerEvent, Q2PrintLevel};
use qa_content::q2::rerelease::types::{Q2LocalizedPrintLevel, Q2RereleaseEvent};
use qa_core::identity::{ActorId, SeatId};
use qa_core::math::{vec2, vec4};
use thiserror::Error;

use self::fog::RereleaseFog;
use crate::bootstrap::q2_localization::{q2_localized_text, Q2LocArg, Q2LocEntry, Q2LocalizationCatalog};

/// Seat binding for rerelease presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleasePresentationSeat {
    /// Presenting seat.
    pub seat: SeatId,
    /// Seat actor.
    pub actor: ActorId,
    /// Initial language.
    pub language: Option<String>,
}

/// Absorbed `SimulationPresentationEvent` pick: only the `q2-rerelease` and
/// `q2-player` sources this presentation consumes (donor
/// `src/app/bootstrap/simulation/types.ts`, out of scope).
#[derive(Debug, Clone, PartialEq)]
pub struct RereleasePresentationEvent {
    /// Content identity.
    pub content: ContentId,
    /// Event time in seconds.
    pub seconds: f64,
    /// Presentation sequence.
    pub sequence: i64,
    /// Seat-actor recipient, when targeted.
    pub recipient: Option<ActorId>,
    /// Source entity, when set.
    pub source_entity: Option<i32>,
    /// Event payload.
    pub kind: RereleasePresentationEventKind,
}

/// Consumed presentation sources.
#[derive(Debug, Clone, PartialEq)]
pub enum RereleasePresentationEventKind {
    /// Rerelease event.
    Rerelease(Q2RereleaseEvent),
    /// Player event.
    Player(Q2PlayerEvent),
}

/// Absorbed `ApplicationAssets` provider pick used by rerelease presentation
/// (donor `src/app/bootstrap/assets.ts`, out of scope): content families,
/// localization mounts, and sky-face images with the missing-texture fallback
/// applied by the host.
pub trait RereleasePresentationProvider: 'static {
    /// Content family for language-binding eligibility.
    fn family(&self, content: &ContentId) -> Option<GameFamily>;
    /// Localization file names below `localization/` ending in `.txt`.
    fn list_localization_files(&self, content: &ContentId) -> Vec<String>;
    /// Open a localization file, returning its bytes.
    fn open_localization(&self, content: &ContentId, path: &str) -> Option<Vec<u8>>;
    /// Load one sky-face image, falling back to the missing texture.
    fn load_sky_face(&self, content: &ContentId, path: &str) -> RendererImage;
}

/// Localizer closing over one seat catalog: `(text, args) -> localized`.
pub type RereleaseLocalizer = Box<dyn Fn(&str, &[String]) -> String>;

/// Rerelease presentation error.
#[derive(Debug, Error)]
pub enum RereleasePresentationError {
    /// Localization seat is unknown.
    #[error("Unknown localization seat")]
    UnknownSeat,
    /// Language setting must be a choice row.
    #[error("Language setting must be a choice")]
    LanguageNotChoice,
    /// Presentation content is unknown.
    #[error("Unknown presentation content: {0}")]
    UnknownContent(String),
    /// Language selection failed.
    #[error("Language selection failed: {0}")]
    SelectionFailed(String),
    /// Story font belongs to another seat.
    #[error("Story font belongs to another seat")]
    StorySeatMismatch,
    /// Text layout or draw failure.
    #[error(transparent)]
    Text(#[from] ClientError),
}

/// Maximum localization key/token sizes (donor `MAX_LOC_KEY`,
/// `MAX_LOC_FORMAT`, `MAX_STRING_CHARS`).
const MAX_LOC_KEY: usize = 64;
const MAX_LOC_FORMAT: usize = 1024;
const MAX_LOC_TOKEN: usize = 1024;
/// Maximum format arguments (donor `MAX_LOC_ARGS`).
const MAX_LOC_ARGS: usize = 8;

/// Parsed `loc_*.txt` strings for one seat and content.
///
/// The donor reads these through `LocalizationCatalog`; `LocalizationTable`
/// keeps `find` private (see `crate::bootstrap::q2_localization`), so this
/// adapter replicates `Loc_ParseInto`/`Loc_Parse` exactly, including the
/// tokenizer's comment and escape rules. One adapter doubles as the
/// [`LocalizationView`] behind [`NativeLanguageSettings`], so language
/// selection loads directly into the strings the presentation localizes.
#[derive(Debug, Clone, Default)]
pub struct RereleaseStrings {
    last_duplicate_wins: bool,
    entries: HashMap<String, Q2LocEntry>,
}

impl RereleaseStrings {
    fn new(last_duplicate_wins: bool) -> Self {
        Self {
            last_duplicate_wins,
            entries: HashMap::new(),
        }
    }

    fn reload(&mut self, bytes: Option<&[u8]>) {
        self.entries.clear();
        let Some(bytes) = bytes else { return };
        for (key, entry) in parse_loc_entries(&String::from_utf8_lossy(bytes)) {
            if self.last_duplicate_wins || !self.entries.contains_key(&key) {
                self.entries.insert(key, entry);
            }
        }
    }

    fn merge(&mut self, bytes: &[u8]) {
        let mut seen = HashSet::new();
        for (key, entry) in parse_loc_entries(&String::from_utf8_lossy(bytes)) {
            if self.last_duplicate_wins || !seen.contains(&key) {
                seen.insert(key.clone());
                self.entries.insert(key, entry);
            }
        }
    }
}

impl LocalizationView for RereleaseStrings {
    fn load_ordered(&mut self, primary: LocLoadTier, fallback: LocLoadTier) {
        let tier = if primary.base.is_some() { &primary } else { &fallback };
        self.reload(tier.base.as_deref());
        for mods in &tier.mods {
            self.merge(mods);
        }
    }
}

impl Q2LocalizationCatalog for RereleaseStrings {
    fn find(&self, key: &str) -> Option<&Q2LocEntry> {
        self.entries.get(key)
    }

    fn localize(&self, text: &str, args: &[String]) -> String {
        let format = text
            .strip_prefix('$')
            .and_then(|key| self.entries.get(key))
            .map(|entry| entry.format.as_str())
            .unwrap_or(text);
        let mut out = format.to_string();
        for (index, arg) in args.iter().enumerate() {
            out = out.replace(&format!("{{{index}}}"), arg);
        }
        out
    }
}

/// `comParseToken` cursor over decoded file text.
struct LocTokenizer<'a> {
    chars: &'a [char],
    index: usize,
}

impl LocTokenizer<'_> {
    fn peek(&self, offset: usize) -> char {
        self.chars.get(self.index + offset).copied().unwrap_or('\0')
    }

    fn escape(&mut self) -> Option<char> {
        let code = self.peek(0);
        self.index += 1;
        if code == '\0' {
            return None;
        }
        Some(match code {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            _ => code,
        })
    }

    fn token(&mut self, size: usize, escape: bool) -> String {
        loop {
            let c = self.peek(0);
            if c == '\0' {
                return String::new();
            }
            if c == '/' && self.peek(1) == '/' {
                self.index += 2;
                while self.peek(0) != '\0' && self.peek(0) != '\n' {
                    self.index += 1;
                }
                continue;
            }
            if c == '/' && self.peek(1) == '*' {
                self.index += 2;
                while self.peek(0) != '\0' {
                    if self.peek(0) == '*' && self.peek(1) == '/' {
                        self.index += 2;
                        break;
                    }
                    self.index += 1;
                }
                continue;
            }
            if (c as u32) <= 32 {
                self.index += 1;
                continue;
            }
            break;
        }
        if self.peek(0) == '=' {
            self.index += 1;
            return "=".to_string();
        }
        if self.peek(0) == '"' {
            self.index += 1;
            let mut out = String::new();
            loop {
                let c = self.peek(0);
                self.index += 1;
                if c == '"' || c == '\0' {
                    return truncate_token(&out, size);
                }
                if c == '\\' && escape {
                    let Some(escaped) = self.escape() else {
                        return truncate_token(&out, size);
                    };
                    out.push(escaped);
                } else {
                    out.push(c);
                }
            }
        }
        // Bare tokens never interpret escapes: the donor peeks the backslash
        // without consuming it, so its escape probe always reads the
        // backslash itself back.
        let mut out = String::new();
        loop {
            let c = self.peek(0);
            if (c as u32) <= 32 || c == '=' || c == '\0' {
                return truncate_token(&out, size);
            }
            out.push(c);
            self.index += 1;
        }
    }
}

/// Donor `strlcpy` truncation: `size - 1` chars, then `size - 1` bytes at a
/// character boundary.
fn truncate_token(text: &str, size: usize) -> String {
    let capped: String = text.chars().take(size.saturating_sub(1)).collect();
    let mut end = size.saturating_sub(1).min(capped.len());
    let bytes = capped.as_bytes();
    while end > 0 && end < bytes.len() && bytes[end] & 0xc0 == 0x80 {
        end -= 1;
    }
    capped[..end].to_string()
}

/// Donor `Loc_Parse`: argument slots with byte offsets (the donor slices by
/// UTF-16 units; byte offsets match for ASCII and stay on boundaries).
fn parse_format_args(format: &str) -> Option<Vec<Q2LocArg>> {
    let chars: Vec<char> = format.chars().collect();
    let mut bytes: Vec<usize> = chars
        .iter()
        .scan(0usize, |offset, c| {
            let start = *offset;
            *offset += c.len_utf8();
            Some(start)
        })
        .collect();
    bytes.push(format.len());
    let mut index_state: i32 = 0;
    let mut rover: usize = 0;
    let mut args = Vec::new();
    loop {
        if rover >= chars.len() {
            break;
        }
        if chars[rover] == '{' {
            let arg_start = rover;
            rover += 1;
            if rover < chars.len() && chars[rover] == '{' {
                continue;
            }
            if args.len() == MAX_LOC_ARGS {
                return None;
            }
            let mut end_ptr = rover;
            while end_ptr < chars.len() && chars[end_ptr].is_ascii_digit() {
                end_ptr += 1;
            }
            let arg_index: usize;
            if end_ptr == rover {
                if index_state == -1 {
                    return None;
                }
                arg_index = (index_state as usize) & 0xff;
                index_state += 1;
            } else {
                if index_state > 0 {
                    return None;
                }
                // `parseInt(digits) & 0xff` for non-negative values is the
                // value mod 256, computed without overflow.
                let mut value: usize = 0;
                for c in &chars[rover..end_ptr] {
                    value = (value * 10 + (*c as usize - '0' as usize)) % 256;
                }
                arg_index = value;
                index_state = -1;
            }
            rover = end_ptr - 1;
            let arg_end = loop {
                if rover >= chars.len() {
                    return None;
                }
                rover += 1;
                if rover >= chars.len() || chars[rover] != '}' {
                    continue;
                }
                let arg_end = rover;
                rover += 1;
                if rover < chars.len() && chars[rover] == '}' {
                    continue;
                }
                break arg_end;
            };
            args.push(Q2LocArg {
                arg_index,
                start: bytes[arg_start],
                end: bytes[arg_end + 1],
            });
        } else {
            rover += 1;
        }
    }
    args.sort_by_key(|arg| arg.start);
    Some(args)
}

/// Donor `Loc_ParseInto` with no platform selected: platform-gated entries
/// never load and malformed formats are skipped.
fn parse_loc_entries(text: &str) -> Vec<(String, Q2LocEntry)> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokenizer = LocTokenizer {
        chars: &chars,
        index: 0,
    };
    let mut out = Vec::new();
    loop {
        let key = tokenizer.token(MAX_LOC_KEY, false);
        if key.is_empty() {
            break;
        }
        let mut equals = tokenizer.token(MAX_LOC_TOKEN, false);
        if equals.is_empty() {
            break;
        }
        let mut platform_spec = false;
        if equals.starts_with('<') {
            platform_spec = true;
            while !equals.is_empty() && !equals.ends_with('>') {
                equals = tokenizer.token(MAX_LOC_TOKEN, false);
            }
            equals = tokenizer.token(MAX_LOC_TOKEN, false);
        }
        if equals != "=" {
            break;
        }
        let format = tokenizer.token(MAX_LOC_FORMAT, true);
        let Some(arguments) = parse_format_args(&format) else {
            continue;
        };
        if platform_spec {
            continue;
        }
        out.push((key, Q2LocEntry { format, arguments }));
    }
    out
}

/// Player name tokens resolve after localized argument expansion, as in
/// `CL_ParseLocPrint`.
#[must_use]
pub fn q2_player_name_tokens(text: &str, names: &HashMap<i32, String>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    let mut literal_start = 0;
    while index < bytes.len() {
        let rest = &bytes[index..];
        if rest.len() > 3 && rest.starts_with(b"##P") && rest[3].is_ascii_digit() {
            out.push_str(&text[literal_start..index]);
            let mut end = index + 4;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            let number: f64 = text[index + 3..end].parse().unwrap_or(f64::NAN);
            let slot = if number.fract() == 0.0 && number >= f64::from(i32::MIN) && number <= f64::from(i32::MAX) {
                Some(number as i32)
            } else {
                None
            };
            if let Some(name) = slot.and_then(|slot| names.get(&slot)) {
                out.push_str(name);
            }
            index = end;
            literal_start = end;
        } else {
            index += 1;
        }
    }
    out.push_str(&text[literal_start..]);
    out
}

/// Sky source identity shared by the global and per-seat skies.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SkySource {
    content: ContentId,
    name: String,
}

/// Per-seat sky override.
#[derive(Debug, Clone, PartialEq)]
struct SeatSky {
    source: SkySource,
    view: Q2SkyView,
}

/// Story text with its localization source for refreshes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StorySource {
    content: ContentId,
    text: String,
}

/// Language settings plus the strings they load into.
struct SeatCatalog {
    settings: NativeLanguageSettings,
    strings: Rc<RefCell<RereleaseStrings>>,
}

/// Per-seat presentation state.
struct SeatState {
    binding: RereleasePresentationSeat,
    catalogs: HashMap<ContentId, SeatCatalog>,
    language: String,
    fog: RereleaseFog,
    fog_received: bool,
    story: String,
    story_source: Option<StorySource>,
    hidden_items: HashSet<ActorId>,
    names: HashMap<i32, String>,
    sky: Option<SeatSky>,
}

impl SeatState {
    fn new(binding: RereleasePresentationSeat) -> Self {
        let language = binding.language.clone().unwrap_or_else(|| "english".to_string());
        Self {
            binding,
            catalogs: HashMap::new(),
            language,
            fog: RereleaseFog::new(),
            fog_received: false,
            story: String::new(),
            story_source: None,
            hidden_items: HashSet::new(),
            names: HashMap::new(),
            sky: None,
        }
    }
}

/// Seat fog and sky for the world view.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseSeatView {
    /// Interpolated fog, once received.
    pub q2_fog: Option<Q2Fog>,
    /// Seat or global sky override.
    pub q2_sky: Option<Q2SkyView>,
}

/// Reloaded sky images awaiting application (donor `prepareImageRefresh`
/// applies through its returned closure).
pub struct RereleaseImageRefresh {
    next: Option<(SkySource, Q2SkyView)>,
    seats: Vec<(Rc<RefCell<SeatState>>, Option<SeatSky>)>,
}

impl RereleaseImageRefresh {
    /// Swap in the reloaded images and clear the sky cache.
    pub fn apply<P: RereleasePresentationProvider>(self, presentation: &mut ApplicationRereleasePresentation<P>) {
        presentation.skies.clear();
        let (sky_source, sky) = match self.next {
            Some((source, view)) => (Some(source), Some(view)),
            None => (None, None),
        };
        presentation.sky_source = sky_source;
        presentation.sky = sky;
        for (seat, sky) in self.seats {
            seat.borrow_mut().sky = sky;
        }
    }
}

/// Q2 rerelease presentation (donor `ApplicationRereleasePresentation`).
pub struct ApplicationRereleasePresentation<P> {
    provider: Rc<P>,
    seats: Vec<Rc<RefCell<SeatState>>>,
    language_settings: Option<Rc<dyn SettingCvars>>,
    names: HashMap<i32, String>,
    pending: Vec<RereleasePresentationEvent>,
    prints: Vec<RereleasePresentationEvent>,
    skies: HashMap<ContentId, HashMap<String, Vec<RendererImage>>>,
    sky: Option<Q2SkyView>,
    sky_source: Option<SkySource>,
}

impl<P: RereleasePresentationProvider> ApplicationRereleasePresentation<P> {
    /// Presentation over a provider, seat bindings, and optional seat
    /// language cvars.
    pub fn new(
        provider: P,
        seats: Vec<RereleasePresentationSeat>,
        language_settings: Option<Rc<dyn SettingCvars>>,
    ) -> Self {
        Self {
            provider: Rc::new(provider),
            seats: seats
                .into_iter()
                .map(|binding| Rc::new(RefCell::new(SeatState::new(binding))))
                .collect(),
            language_settings,
            names: HashMap::new(),
            pending: Vec::new(),
            prints: Vec::new(),
            skies: HashMap::new(),
            sky: None,
            sky_source: None,
        }
    }

    /// Reload global and per-seat sky images, returning the deferred swap.
    pub fn prepare_image_refresh(&self) -> RereleaseImageRefresh {
        let next = match (&self.sky, &self.sky_source) {
            (Some(sky), Some(source)) => Some((
                source.clone(),
                Q2SkyView {
                    images: Self::load_sky_images(&self.provider, &source.content, &source.name),
                    ..sky.clone()
                },
            )),
            _ => None,
        };
        let seats = self
            .seats
            .iter()
            .map(|seat| {
                let sky = seat.borrow().sky.clone().map(|sky| SeatSky {
                    view: Q2SkyView {
                        images: Self::load_sky_images(&self.provider, &sky.source.content, &sky.source.name),
                        ..sky.view.clone()
                    },
                    source: sky.source.clone(),
                });
                (Rc::clone(seat), sky)
            })
            .collect();
        RereleaseImageRefresh { next, seats }
    }

    /// Republish seat bindings, retaining state for unchanged seats.
    pub fn publish_seats(&mut self, bindings: &[RereleasePresentationSeat]) {
        self.seats = bindings
            .iter()
            .map(|binding| {
                self.seats
                    .iter()
                    .find(|seat| {
                        let state = seat.borrow();
                        state.binding.seat == binding.seat && state.binding.actor == binding.actor
                    })
                    .cloned()
                    .unwrap_or_else(|| Rc::new(RefCell::new(SeatState::new(binding.clone()))))
            })
            .collect();
    }

    /// Receive presentation events, applying visibility and fog at once and
    /// queueing the rest for [`Self::prepare`].
    pub fn receive(&mut self, events: &[RereleasePresentationEvent]) {
        for source in events {
            match &source.kind {
                RereleasePresentationEventKind::Player(Q2PlayerEvent::Userinfo { .. }) => {
                    self.pending.push(source.clone());
                }
                RereleasePresentationEventKind::Player(_) => {}
                RereleasePresentationEventKind::Rerelease(event) => match event {
                    Q2RereleaseEvent::ItemVisibility { actor, item, visible } => {
                        for seat in &self.seats {
                            let mut seat = seat.borrow_mut();
                            if seat.binding.actor == *actor
                                && (source.recipient.is_none()
                                    || source.recipient.as_ref() == Some(&seat.binding.actor))
                            {
                                seat.hidden_items.remove(item);
                                if !visible {
                                    seat.hidden_items.insert(item.clone());
                                }
                            }
                        }
                    }
                    Q2RereleaseEvent::Fog {
                        actor,
                        value,
                        transition_milliseconds,
                    } => {
                        for seat in &self.seats {
                            let mut seat = seat.borrow_mut();
                            if seat.binding.actor == *actor
                                && (source.recipient.is_none()
                                    || source.recipient.as_ref() == Some(&seat.binding.actor))
                            {
                                seat.fog.receive(value, *transition_milliseconds, source.seconds);
                                seat.fog_received = true;
                            }
                        }
                    }
                    Q2RereleaseEvent::Story { .. }
                    | Q2RereleaseEvent::LocalizedPrint { .. }
                    | Q2RereleaseEvent::Sky { .. } => {
                        self.pending.push(source.clone());
                    }
                    _ => {}
                },
            }
        }
    }

    /// Selected language for a seat: the cvar wins when settings exist.
    #[must_use]
    pub fn selected_language(&self, id: &SeatId) -> String {
        match &self.language_settings {
            Some(settings) => read_seat_language(&**settings, id.index()),
            None => self
                .seats
                .iter()
                .find(|seat| seat.borrow().binding.seat == *id)
                .map(|seat| seat.borrow().language.clone())
                .unwrap_or_else(|| "english".to_string()),
        }
    }

    /// Whether an item is visible to an actor.
    #[must_use]
    pub fn item_visible(&self, actor: &ActorId, item: &ActorId) -> bool {
        match self.seats.iter().find(|seat| seat.borrow().binding.actor == *actor) {
            Some(seat) => !seat.borrow().hidden_items.contains(item),
            None => true,
        }
    }

    /// Localizer closing over the seat catalog for one content.
    pub fn source_localizer(
        &mut self,
        id: &SeatId,
        content: &ContentId,
    ) -> Result<RereleaseLocalizer, RereleasePresentationError> {
        let seat = self.seat_by_id(id).ok_or(RereleasePresentationError::UnknownSeat)?;
        let mut state = seat.borrow_mut();
        let strings = Self::catalog(&mut state, &self.provider, self.language_settings.as_ref(), content)?
            .strings
            .clone();
        Ok(Box::new(move |text, args| {
            let guard = strings.borrow();
            let strings: &RereleaseStrings = &guard;
            q2_localized_text(strings, text, args)
        }))
    }

    /// Localize a message, then resolve player-name tokens.
    pub fn localize_message(
        &mut self,
        id: &SeatId,
        content: &ContentId,
        text: &str,
        args: &[String],
    ) -> Result<String, RereleasePresentationError> {
        let seat = self.seat_by_id(id).ok_or(RereleasePresentationError::UnknownSeat)?;
        let mut state = seat.borrow_mut();
        let strings = Self::catalog(&mut state, &self.provider, self.language_settings.as_ref(), content)?
            .strings
            .clone();
        let mut names = self.names.clone();
        names.extend(state.names.clone());
        drop(state);
        let guard = strings.borrow();
        let strings: &RereleaseStrings = &guard;
        Ok(q2_player_name_tokens(&q2_localized_text(strings, text, args), &names))
    }

    /// Language choice row for Q1/Q2 content, if the provider knows it.
    pub fn language_binding(
        &mut self,
        id: &SeatId,
        content: &ContentId,
        failed: Rc<dyn Fn(String)>,
    ) -> Result<Option<SettingBinding>, RereleasePresentationError> {
        if !matches!(self.provider.family(content), Some(GameFamily::Q1 | GameFamily::Q2)) {
            return Ok(None);
        }
        let seat = self.seat_by_id(id).ok_or(RereleasePresentationError::UnknownSeat)?;
        let mut state = seat.borrow_mut();
        let catalog = Self::catalog(&mut state, &self.provider, self.language_settings.as_ref(), content)?;
        let inner = catalog.settings.binding();
        let choices = match inner.kind {
            SettingBindingKind::Choice { choices, .. } => choices,
            _ => return Err(RereleasePresentationError::LanguageNotChoice),
        };
        let index = id.index();
        let read_seat = Rc::clone(&seat);
        let read_settings = self.language_settings.clone();
        let read = Rc::new(move || match &read_settings {
            Some(settings) => read_seat_language(&**settings, index),
            None => read_seat.borrow().language.clone(),
        });
        // The donor throws on invalid choices and reloads the catalog in the
        // background; with nowhere to throw, write failures route to the
        // failed sink like `NativeLanguageSettings` row writes.
        let write_seat = Rc::clone(&seat);
        let write_settings = self.language_settings.clone();
        let write_provider = Rc::clone(&self.provider);
        let write_content = content.clone();
        let write_choices = Rc::clone(&choices);
        let write = Rc::new(move |value: &str| {
            if !write_choices().iter().any(|choice| choice.id == value) {
                failed(format!("Language is not installed: {value}"));
                return;
            }
            let mut state = write_seat.borrow_mut();
            if let Some(settings) = &write_settings {
                if let Err(error) = write_seat_language(&**settings, index, value) {
                    failed(error.to_string());
                    return;
                }
            } else {
                state.language = value.to_string();
            }
            state.catalogs.clear();
            if let Err(error) = Self::catalog(&mut state, &write_provider, write_settings.as_ref(), &write_content) {
                failed(error.to_string());
            }
        });
        Ok(Some(SettingBinding {
            id: inner.id,
            label: inner.label,
            category: inner.category,
            enabled: inner.enabled,
            kind: SettingBindingKind::Choice { read, write, choices },
        }))
    }

    /// Process queued userinfo, sky, story, and print events, then refresh
    /// stories against the current language.
    pub fn prepare(&mut self) -> Result<(), RereleasePresentationError> {
        for source in std::mem::take(&mut self.pending) {
            let RereleasePresentationEvent {
                content,
                seconds,
                sequence,
                recipient,
                source_entity,
                kind,
            } = source;
            match kind {
                RereleasePresentationEventKind::Player(Q2PlayerEvent::Userinfo { slot, name, .. }) => match recipient {
                    None => {
                        self.names.insert(slot, name);
                        for seat in &self.seats {
                            seat.borrow_mut().names.remove(&slot);
                        }
                    }
                    Some(recipient) => {
                        for seat in &self.seats {
                            let mut seat = seat.borrow_mut();
                            if recipient == seat.binding.actor {
                                seat.names.insert(slot, name.clone());
                            }
                        }
                    }
                },
                RereleasePresentationEventKind::Rerelease(Q2RereleaseEvent::Sky {
                    name,
                    rotation,
                    auto_rotate,
                    axis,
                }) => {
                    let sky_source = SkySource {
                        content: content.clone(),
                        name,
                    };
                    let sky = Q2SkyView {
                        images: self.sky_images(&sky_source.content, &sky_source.name),
                        rotation: rotation as f32,
                        auto_rotate: rotation != 0.0 && auto_rotate,
                        axis,
                    };
                    match recipient {
                        None => {
                            self.sky_source = Some(sky_source);
                            self.sky = Some(sky);
                            for seat in &self.seats {
                                seat.borrow_mut().sky = None;
                            }
                        }
                        Some(recipient) => {
                            for seat in &self.seats {
                                let mut seat = seat.borrow_mut();
                                if recipient == seat.binding.actor {
                                    seat.sky = Some(SeatSky {
                                        source: sky_source.clone(),
                                        view: sky.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
                RereleasePresentationEventKind::Rerelease(
                    event @ (Q2RereleaseEvent::Story { .. } | Q2RereleaseEvent::LocalizedPrint { .. }),
                ) => {
                    let seats: Vec<Rc<RefCell<SeatState>>> = self.seats.to_vec();
                    for seat in seats {
                        let (seat_id, seat_actor) = {
                            let seat = seat.borrow();
                            (seat.binding.seat.clone(), seat.binding.actor.clone())
                        };
                        if recipient.as_ref().is_some_and(|recipient| *recipient != seat_actor) {
                            continue;
                        }
                        let (text, args) = match &event {
                            Q2RereleaseEvent::Story { text } => (text.clone(), Vec::new()),
                            Q2RereleaseEvent::LocalizedPrint {
                                actor,
                                level: _,
                                text,
                                args,
                            } => {
                                if actor.as_ref().is_some_and(|actor| *actor != seat_actor) {
                                    continue;
                                }
                                (text.clone(), args.clone())
                            }
                            _ => continue,
                        };
                        let localized = self.localize_message(&seat_id, &content, &text, &args)?;
                        match &event {
                            Q2RereleaseEvent::Story { .. } => {
                                let mut seat = seat.borrow_mut();
                                seat.story = localized;
                                seat.story_source = Some(StorySource {
                                    content: content.clone(),
                                    text,
                                });
                            }
                            Q2RereleaseEvent::LocalizedPrint { level, .. } => {
                                self.prints.push(RereleasePresentationEvent {
                                    content: content.clone(),
                                    seconds,
                                    sequence,
                                    recipient: recipient.clone(),
                                    source_entity,
                                    kind: RereleasePresentationEventKind::Player(Q2PlayerEvent::Print {
                                        target: Some(seat_actor),
                                        level: print_level(level),
                                        text: localized,
                                    }),
                                });
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        let seats: Vec<Rc<RefCell<SeatState>>> = self.seats.to_vec();
        for seat in seats {
            let (seat_id, story_source) = {
                let seat = seat.borrow();
                (seat.binding.seat.clone(), seat.story_source.clone())
            };
            if let Some(source) = story_source {
                let localized = self.localize_message(&seat_id, &source.content, &source.text, &[])?;
                seat.borrow_mut().story = localized;
            }
        }
        Ok(())
    }

    /// Drain localized print events.
    #[must_use]
    pub fn drain_prints(&mut self) -> Vec<RereleasePresentationEvent> {
        std::mem::take(&mut self.prints)
    }

    /// Whether an actor has an active story.
    #[must_use]
    pub fn story_active(&self, actor: &ActorId) -> bool {
        self.seats.iter().any(|seat| {
            let seat = seat.borrow();
            seat.binding.actor == *actor && !seat.story.is_empty()
        })
    }

    /// Seat fog and sky for the world view.
    #[must_use]
    pub fn view(&self, actor: &ActorId, seconds: f64) -> RereleaseSeatView {
        let seat = self.seats.iter().find(|seat| seat.borrow().binding.actor == *actor);
        let fog = seat.and_then(|seat| {
            let seat = seat.borrow();
            seat.fog_received.then(|| seat.fog.current(seconds))
        });
        let sky = seat
            .and_then(|seat| seat.borrow().sky.clone().map(|sky| sky.view))
            .or_else(|| self.sky.clone());
        RereleaseSeatView {
            q2_fog: fog,
            q2_sky: sky,
        }
    }

    /// Draw an actor's story centered, as in `cg_screen.cpp`.
    pub fn draw_story(
        &self,
        actor: &ActorId,
        draw: &mut Draw2D,
        text: &SeatTextPresentation,
        scale: f32,
    ) -> Result<(), RereleasePresentationError> {
        let seat = self.seats.iter().find(|seat| seat.borrow().binding.actor == *actor);
        let Some(seat) = seat else { return Ok(()) };
        let seat = seat.borrow();
        if seat.story.is_empty() {
            return Ok(());
        }
        if text.seat != seat.binding.seat {
            return Err(RereleasePresentationError::StorySeatMismatch);
        }
        let options = TextLayoutOptionsWithoutFont {
            text: &seat.story,
            scale,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            color_codes: ColorCodes::Literal,
            force_color: false,
            alternate: false,
            max_width: None,
            align: TextAlign::Center,
            line_height: None,
            max_glyphs: None,
            tab_columns: 4,
        };
        let layout = text.layout(options.clone())?;
        let origin = vec2(
            (draw.width() - layout.width) / 2.0,
            (draw.height() - layout.height) / 2.0,
        );
        text.draw(
            draw,
            TextLayoutOptionsWithoutFont {
                max_width: Some(layout.width.max(1.0)),
                ..options
            },
            origin,
            scale,
        )?;
        Ok(())
    }

    fn seat_by_id(&self, id: &SeatId) -> Option<Rc<RefCell<SeatState>>> {
        self.seats
            .iter()
            .find(|seat| seat.borrow().binding.seat == *id)
            .cloned()
    }

    fn sync_language(seat: &mut SeatState, settings: Option<&Rc<dyn SettingCvars>>) {
        let selected = match settings {
            Some(settings) => read_seat_language(&**settings, seat.binding.seat.index()),
            None => seat.language.clone(),
        };
        if selected != seat.language {
            seat.language = selected;
            seat.catalogs.clear();
        }
    }

    /// Seat catalog for one content, rebuilt when the language changed.
    fn catalog<'s>(
        seat: &'s mut SeatState,
        provider: &Rc<P>,
        settings: Option<&Rc<dyn SettingCvars>>,
        content: &ContentId,
    ) -> Result<&'s mut SeatCatalog, RereleasePresentationError> {
        Self::sync_language(seat, settings);
        let selection = seat.language.clone();
        let fresh = seat.catalogs.contains_key(content)
            && match settings {
                Some(settings) => read_seat_language(&**settings, seat.binding.seat.index()) == selection,
                None => true,
            };
        if !fresh {
            seat.catalogs.remove(content);
            let family = provider
                .family(content)
                .ok_or_else(|| RereleasePresentationError::UnknownContent(content.as_str().to_string()))?;
            let profile = if family == GameFamily::Q1 {
                LocalizationProfile::Q1Rerelease
            } else {
                LocalizationProfile::Q2Rerelease
            };
            let mut languages: Vec<String> = vec!["english".to_string(), selection.clone()];
            for file in provider.list_localization_files(content) {
                let lowered = file.to_lowercase();
                if let Some(stem) = lowered.strip_prefix("loc_").and_then(|stem| stem.strip_suffix(".txt")) {
                    if !stem.is_empty() && stem.chars().all(|c| c.is_ascii_lowercase()) {
                        languages.push(stem.to_string());
                    }
                }
            }
            languages.sort();
            languages.dedup();
            let mut choices = Vec::with_capacity(languages.len());
            for language in &languages {
                let label = match language.chars().next() {
                    Some(first) => first.to_uppercase().collect::<String>() + &language[first.len_utf8()..],
                    None => String::new(),
                };
                let provider = Rc::clone(provider);
                let content = content.clone();
                let language = language.clone();
                choices.push(LanguageChoice {
                    id: language.clone(),
                    label,
                    load: Rc::new(move || {
                        let open = |name: &str| provider.open_localization(&content, name);
                        let english = language == "english";
                        Ok((
                            LocLoadTier {
                                base: open(&format!("localization/loc_{language}.txt")),
                                mods: open(&format!("localization/loc_{language}_mod.txt"))
                                    .into_iter()
                                    .collect(),
                            },
                            LocLoadTier {
                                base: if english {
                                    None
                                } else {
                                    open("localization/loc_english.txt")
                                },
                                mods: if english {
                                    Vec::new()
                                } else {
                                    open("localization/loc_english_mod.txt").into_iter().collect()
                                },
                            },
                        ))
                    }),
                });
            }
            let strings = Rc::new(RefCell::new(RereleaseStrings::new(matches!(
                profile,
                LocalizationProfile::Q2Rerelease
            ))));
            let view: Rc<RefCell<dyn LocalizationView>> = strings.clone();
            let mut catalog_settings = NativeLanguageSettings::new(view, choices, selection.clone(), Rc::new(|_| {}));
            catalog_settings
                .select(&selection)
                .map_err(RereleasePresentationError::SelectionFailed)?;
            seat.catalogs.insert(
                content.clone(),
                SeatCatalog {
                    settings: catalog_settings,
                    strings,
                },
            );
        }
        seat.catalogs
            .get_mut(content)
            .ok_or_else(|| RereleasePresentationError::UnknownContent(content.as_str().to_string()))
    }

    fn sky_images(&mut self, content: &ContentId, name: &str) -> Vec<RendererImage> {
        if let Some(cached) = self.skies.get(content).and_then(|cache| cache.get(name)) {
            return cached.clone();
        }
        let images = Self::load_sky_images(&self.provider, content, name);
        self.skies
            .entry(content.clone())
            .or_default()
            .insert(name.to_string(), images.clone());
        images
    }

    fn load_sky_images(provider: &P, content: &ContentId, name: &str) -> Vec<RendererImage> {
        SKY_FACE_SUFFIXES
            .iter()
            .map(|suffix| provider.load_sky_face(content, &format!("env/{name}{suffix}")))
            .collect()
    }
}

/// Localized-print level to player-print level.
fn print_level(level: &Q2LocalizedPrintLevel) -> Q2PrintLevel {
    match level {
        Q2LocalizedPrintLevel::Low => Q2PrintLevel::Low,
        Q2LocalizedPrintLevel::Medium => Q2PrintLevel::Medium,
        Q2LocalizedPrintLevel::High => Q2PrintLevel::High,
        Q2LocalizedPrintLevel::Chat => Q2PrintLevel::Chat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use qa_client::render::types::{ImageSource, ResourceOwner};
    use qa_client::text::atlas::{classic_charset, TextFontSelection};
    use qa_client::text::draw2d::{CoordinateSpace, Rect as DrawRect, TextCommandSink};
    use qa_client::ui::settings::CvarView;
    use qa_client::ui::types::CommandDialect;
    use qa_content::q2::rerelease::types::create_q2_fog;
    use qa_core::identity::{IdentityOwner, SessionId};
    use qa_core::math::vec3;

    struct FakeProvider {
        session: SessionId,
        family: GameFamily,
        files: HashMap<String, Vec<u8>>,
        loads: Cell<usize>,
    }

    impl FakeProvider {
        fn new(session: SessionId, files: &[(&str, &str)]) -> Self {
            Self {
                session,
                family: GameFamily::Q2,
                files: files
                    .iter()
                    .map(|(path, text)| (path.to_string(), text.as_bytes().to_vec()))
                    .collect(),
                loads: Cell::new(0),
            }
        }
    }

    impl RereleasePresentationProvider for FakeProvider {
        fn family(&self, _content: &ContentId) -> Option<GameFamily> {
            Some(self.family)
        }

        fn list_localization_files(&self, _content: &ContentId) -> Vec<String> {
            let mut names: Vec<String> = self
                .files
                .keys()
                .filter_map(|path| path.strip_prefix("localization/").map(str::to_string))
                .filter(|name| name.ends_with(".txt"))
                .collect();
            names.sort();
            names
        }

        fn open_localization(&self, _content: &ContentId, path: &str) -> Option<Vec<u8>> {
            self.files.get(path).cloned()
        }

        fn load_sky_face(&self, _content: &ContentId, path: &str) -> RendererImage {
            let ordinal = self.loads.get() as u32;
            self.loads.set(ordinal as usize + 1);
            RendererImage {
                owner: ResourceOwner::new(1, self.session.clone(), 0),
                ordinal,
                source: ImageSource::Resource {
                    requested_path: path.to_string(),
                },
                width: 64,
                height: 64,
            }
        }
    }

    struct FakeCvars {
        values: RefCell<HashMap<String, String>>,
    }

    impl SettingCvars for FakeCvars {
        fn dialect(&self) -> CommandDialect {
            CommandDialect::Q2Classic
        }

        fn find(&self, name: &str) -> Option<CvarView> {
            self.values.borrow().get(name).map(|value| CvarView {
                value: value.clone(),
                latched_value: None,
                reset_value: value.clone(),
                flags: 0,
            })
        }

        fn set(&self, name: &str, value: &str) {
            self.values.borrow_mut().insert(name.to_string(), value.to_string());
        }

        fn variable_value(&self, name: &str) -> f32 {
            self.values
                .borrow()
                .get(name)
                .and_then(|value| value.parse().ok())
                .unwrap_or(0.0)
        }
    }

    fn owner() -> IdentityOwner {
        IdentityOwner::create("rerelease").expect("owner")
    }

    fn content() -> ContentId {
        ContentId("test-content".to_string())
    }

    fn binding(owner: &IdentityOwner, seat: u32, slot: u32) -> RereleasePresentationSeat {
        RereleasePresentationSeat {
            seat: owner.seat(seat),
            actor: owner.actor(slot, 0),
            language: None,
        }
    }

    fn presentation(owner: &IdentityOwner, files: &[(&str, &str)]) -> ApplicationRereleasePresentation<FakeProvider> {
        let provider = FakeProvider::new(owner.session().clone(), files);
        ApplicationRereleasePresentation::new(provider, vec![binding(owner, 0, 1), binding(owner, 1, 2)], None)
    }

    fn event(kind: RereleasePresentationEventKind) -> RereleasePresentationEvent {
        RereleasePresentationEvent {
            content: content(),
            seconds: 1.0,
            sequence: 7,
            recipient: None,
            source_entity: None,
            kind,
        }
    }

    const ENGLISH: &str = r#"
        // English strings
        /* block comment */ STORY = "The $name cometh"
        GREETING = "Hello {0}!"
        ESCAPED = "tab\there"
        "#;

    #[test]
    fn name_tokens_replace_and_preserve_unicode() {
        let names = HashMap::from([(1, "Bob".to_string())]);
        assert_eq!(q2_player_name_tokens("Hi ##P1!", &names), "Hi Bob!");
        assert_eq!(q2_player_name_tokens("Hi ##P9!", &names), "Hi !");
        assert_eq!(q2_player_name_tokens("Héllo ##P1 ✓ ##P", &names), "Héllo Bob ✓ ##P");
        assert_eq!(
            q2_player_name_tokens("##P007", &HashMap::from([(7, "x".to_string())])),
            "x"
        );
    }

    #[test]
    fn loc_parser_reads_keys_escapes_comments() {
        let mut strings = RereleaseStrings::new(true);
        strings.reload(Some(ENGLISH.as_bytes()));
        assert_eq!(strings.find("STORY").expect("story").format, "The $name cometh");
        assert_eq!(strings.find("ESCAPED").expect("escaped").format, "tab\there");
        let greeting = strings.find("GREETING").expect("greeting");
        assert_eq!(greeting.arguments.len(), 1);
        assert_eq!(greeting.arguments[0].arg_index, 0);
    }

    #[test]
    fn loc_parser_skips_platform_and_bad_formats() {
        let mut strings = RereleaseStrings::new(true);
        strings.reload(Some(b"GATED <ps> = \"no\"\nPLAIN = \"yes\"\nMIXED = \"{0} and {}\"\n"));
        assert!(strings.find("GATED").is_none());
        assert_eq!(strings.find("PLAIN").expect("plain").format, "yes");
        assert!(strings.find("MIXED").is_none());
    }

    #[test]
    fn loc_duplicates_follow_profile() {
        let bytes = b"KEY = \"first\"\nKEY = \"second\"\n";
        let mut q1 = RereleaseStrings::new(false);
        q1.reload(Some(bytes));
        assert_eq!(q1.find("KEY").expect("q1").format, "first");
        let mut q2 = RereleaseStrings::new(true);
        q2.reload(Some(bytes));
        assert_eq!(q2.find("KEY").expect("q2").format, "second");
    }

    #[test]
    fn loc_tiers_fall_back_and_merge_mods() {
        let mut strings = RereleaseStrings::new(true);
        strings.load_ordered(
            LocLoadTier {
                base: None,
                mods: vec![],
            },
            LocLoadTier {
                base: Some(b"BASE = \"base\"\nSHARED = \"base\"\n".to_vec()),
                mods: vec![b"SHARED = \"mod\"\n".to_vec()],
            },
        );
        assert_eq!(strings.find("BASE").expect("base").format, "base");
        assert_eq!(strings.find("SHARED").expect("shared").format, "mod");
    }

    #[test]
    fn story_sets_and_refreshes_on_language_change() {
        let owner = owner();
        let mut app = presentation(
            &owner,
            &[
                ("localization/loc_english.txt", "STORY = \"English story\"\n"),
                ("localization/loc_french.txt", "STORY = \"Histoire\"\n"),
            ],
        );
        app.receive(&[event(RereleasePresentationEventKind::Rerelease(
            Q2RereleaseEvent::Story {
                text: "$STORY".to_string(),
            },
        ))]);
        app.prepare().expect("prepare");
        assert!(app.story_active(&owner.actor(1, 0)));
        assert_eq!(app.seats[0].borrow().story, "English story");
        let failed = Rc::new(|_: String| {});
        let binding = app
            .language_binding(&owner.seat(0), &content(), failed)
            .expect("binding")
            .expect("choice");
        let SettingBindingKind::Choice { write, read, .. } = &binding.kind else {
            panic!("choice row");
        };
        write("french");
        assert_eq!(read(), "french");
        app.prepare().expect("refresh");
        assert_eq!(app.seats[0].borrow().story, "Histoire");
        // The other seat keeps English.
        assert_eq!(app.seats[1].borrow().story, "English story");
    }

    #[test]
    fn localized_print_drains_with_level_and_envelope() {
        let owner = owner();
        let mut app = presentation(&owner, &[("localization/loc_english.txt", "KILL = \"{0} fragged\"\n")]);
        let actor = owner.actor(1, 0);
        let mut source = event(RereleasePresentationEventKind::Rerelease(
            Q2RereleaseEvent::LocalizedPrint {
                actor: Some(actor.clone()),
                level: Q2LocalizedPrintLevel::High,
                text: "$KILL".to_string(),
                args: vec!["Bob".to_string()],
            },
        ));
        source.recipient = Some(actor.clone());
        source.source_entity = Some(9);
        app.receive(&[source]);
        app.prepare().expect("prepare");
        let prints = app.drain_prints();
        assert_eq!(prints.len(), 1);
        let print = &prints[0];
        assert_eq!((print.sequence, print.source_entity), (7, Some(9)));
        assert_eq!(print.recipient, Some(actor.clone()));
        let RereleasePresentationEventKind::Player(Q2PlayerEvent::Print { target, level, text }) = &print.kind else {
            panic!("print event");
        };
        assert_eq!(
            (target, level, text.as_str()),
            (&Some(actor), &Q2PrintLevel::High, "Bob fragged")
        );
        assert!(app.drain_prints().is_empty());
    }

    #[test]
    fn userinfo_names_scope_globally_and_per_seat() {
        let owner = owner();
        let mut app = presentation(&owner, &[("localization/loc_english.txt", "")]);
        let userinfo = |slot: i32, name: &str| {
            event(RereleasePresentationEventKind::Player(Q2PlayerEvent::Userinfo {
                actor: owner.actor(1, 0),
                slot,
                name: name.to_string(),
                skin: String::new(),
            }))
        };
        let mut targeted = userinfo(3, "Seat");
        targeted.recipient = Some(owner.actor(2, 0));
        app.receive(&[userinfo(1, "Global"), targeted]);
        app.prepare().expect("prepare");
        assert_eq!(app.names.get(&1).map(String::as_str), Some("Global"));
        assert!(app.seats[0].borrow().names.is_empty());
        assert_eq!(app.seats[1].borrow().names.get(&3).map(String::as_str), Some("Seat"));
        // A global update for the same slot clears seat overrides.
        app.receive(&[userinfo(3, "Now global")]);
        app.prepare().expect("prepare");
        assert!(app.seats[1].borrow().names.is_empty());
    }

    #[test]
    fn item_visibility_filters_recipient() {
        let owner = owner();
        let mut app = presentation(&owner, &[("localization/loc_english.txt", "")]);
        let (actor, other, item) = (owner.actor(1, 0), owner.actor(2, 0), owner.actor(9, 0));
        let mut hide = event(RereleasePresentationEventKind::Rerelease(
            Q2RereleaseEvent::ItemVisibility {
                actor: actor.clone(),
                item: item.clone(),
                visible: false,
            },
        ));
        hide.recipient = Some(actor.clone());
        app.receive(&[hide]);
        assert!(!app.item_visible(&actor, &item));
        assert!(app.item_visible(&other, &item));
        app.receive(&[event(RereleasePresentationEventKind::Rerelease(
            Q2RereleaseEvent::ItemVisibility {
                actor: actor.clone(),
                item: item.clone(),
                visible: true,
            },
        ))]);
        assert!(app.item_visible(&actor, &item));
        assert!(app.item_visible(&owner.actor(42, 0), &item));
    }

    #[test]
    fn fog_received_shows_in_view() {
        let owner = owner();
        let mut app = presentation(&owner, &[("localization/loc_english.txt", "")]);
        let actor = owner.actor(1, 0);
        let mut state = create_q2_fog();
        state.fog.density = 0.5;
        app.receive(&[event(RereleasePresentationEventKind::Rerelease(
            Q2RereleaseEvent::Fog {
                actor: actor.clone(),
                value: state,
                transition_milliseconds: 0.0,
            },
        ))]);
        let view = app.view(&actor, 1.0);
        assert!((view.q2_fog.expect("fog").density - 0.5).abs() < 1e-6);
        assert!(view.q2_sky.is_none());
        assert!(app.view(&owner.actor(2, 0), 1.0).q2_fog.is_none());
    }

    #[test]
    fn sky_global_and_seat_override() {
        let owner = owner();
        let mut app = presentation(&owner, &[("localization/loc_english.txt", "")]);
        let sky = |name: &str| {
            event(RereleasePresentationEventKind::Rerelease(Q2RereleaseEvent::Sky {
                name: name.to_string(),
                rotation: 2.0,
                auto_rotate: true,
                axis: vec3(0.0, 0.0, 1.0),
            }))
        };
        app.receive(&[sky("global")]);
        app.prepare().expect("prepare");
        let view = app.view(&owner.actor(1, 0), 0.0);
        let global = view.q2_sky.expect("global sky");
        assert_eq!(global.images.len(), 6);
        assert!(global.auto_rotate);
        let mut targeted = sky("seat");
        targeted.recipient = Some(owner.actor(2, 0));
        app.receive(&[targeted]);
        app.prepare().expect("prepare");
        let seat_view = app.view(&owner.actor(2, 0), 0.0).q2_sky.expect("seat sky");
        let other_view = app.view(&owner.actor(1, 0), 0.0).q2_sky.expect("global sky");
        assert_ne!(seat_view.images[0].ordinal, other_view.images[0].ordinal);
        // Zero rotation never auto-rotates.
        app.receive(&[event(RereleasePresentationEventKind::Rerelease(
            Q2RereleaseEvent::Sky {
                name: "still".to_string(),
                rotation: 0.0,
                auto_rotate: true,
                axis: vec3(0.0, 0.0, 1.0),
            },
        ))]);
        app.prepare().expect("prepare");
        assert!(!app.view(&owner.actor(1, 0), 0.0).q2_sky.expect("still").auto_rotate);
    }

    #[test]
    fn language_binding_lists_choices_and_rejects_unknown() {
        let owner = owner();
        let mut app = presentation(
            &owner,
            &[
                ("localization/loc_english.txt", ""),
                ("localization/loc_french.txt", ""),
                ("localization/notes.md", ""),
            ],
        );
        let failures = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&failures);
        let binding = app
            .language_binding(
                &owner.seat(0),
                &content(),
                Rc::new(move |error| sink.borrow_mut().push(error)),
            )
            .expect("binding")
            .expect("choice");
        let SettingBindingKind::Choice { choices, write, read } = &binding.kind else {
            panic!("choice row");
        };
        let ids: Vec<String> = choices().iter().map(|choice| choice.id.clone()).collect();
        assert_eq!(ids, vec!["english".to_string(), "french".to_string()]);
        assert_eq!(read(), "english");
        write("klingon");
        assert_eq!(failures.borrow().len(), 1);
        assert_eq!(read(), "english");
        write("french");
        assert_eq!(read(), "french");
        assert_eq!(failures.borrow().len(), 1);
    }

    #[test]
    fn language_binding_skips_non_q1_q2() {
        let owner = owner();
        let session = owner.session().clone();
        let mut provider = FakeProvider::new(session, &[("localization/loc_english.txt", "")]);
        provider.family = GameFamily::Q3;
        let mut app = ApplicationRereleasePresentation::new(provider, vec![binding(&owner, 0, 1)], None);
        let bound = app
            .language_binding(&owner.seat(0), &content(), Rc::new(|_| {}))
            .expect("binding");
        assert!(bound.is_none());
    }

    #[test]
    fn language_binding_with_cvars_reads_and_writes() {
        let owner = owner();
        let mut app = ApplicationRereleasePresentation::new(
            FakeProvider::new(
                owner.session().clone(),
                &[
                    ("localization/loc_english.txt", ""),
                    ("localization/loc_french.txt", ""),
                ],
            ),
            vec![binding(&owner, 0, 1)],
            Some(Rc::new(FakeCvars {
                values: RefCell::new(HashMap::new()),
            })),
        );
        assert_eq!(app.selected_language(&owner.seat(0)), "english");
        let binding = app
            .language_binding(&owner.seat(0), &content(), Rc::new(|_| {}))
            .expect("binding")
            .expect("choice");
        let SettingBindingKind::Choice { write, read, .. } = &binding.kind else {
            panic!("choice row");
        };
        // Only installed languages write through.
        write("klingon");
        assert_eq!(read(), "english");
        write("french");
        assert_eq!(read(), "french");
        assert_eq!(app.selected_language(&owner.seat(0)), "french");
    }

    #[test]
    fn unknown_seats_error() {
        let owner = owner();
        let mut app = presentation(&owner, &[("localization/loc_english.txt", "")]);
        let foreign = owner.seat(9);
        assert!(matches!(
            app.localize_message(&foreign, &content(), "x", &[]),
            Err(RereleasePresentationError::UnknownSeat)
        ));
        assert!(matches!(
            app.source_localizer(&foreign, &content()),
            Err(RereleasePresentationError::UnknownSeat)
        ));
        assert!(matches!(
            app.language_binding(&foreign, &content(), Rc::new(|_| {})),
            Err(RereleasePresentationError::UnknownSeat)
        ));
    }

    #[test]
    fn source_localizer_closes_over_catalog() {
        let owner = owner();
        let mut app = presentation(
            &owner,
            &[("localization/loc_english.txt", "GREETING = \"Hello {0}!\"\n")],
        );
        let localize = app.source_localizer(&owner.seat(0), &content()).expect("localizer");
        assert_eq!(localize("$GREETING", &["Marine".to_string()]), "Hello Marine!");
        assert_eq!(localize("$MISSING", &[]), "$MISSING");
    }

    #[test]
    fn publish_seats_retains_state() {
        let owner = owner();
        let mut app = presentation(&owner, &[("localization/loc_english.txt", "STORY = \"Kept\"\n")]);
        app.receive(&[event(RereleasePresentationEventKind::Rerelease(
            Q2RereleaseEvent::Story {
                text: "$STORY".to_string(),
            },
        ))]);
        app.prepare().expect("prepare");
        app.publish_seats(&[binding(&owner, 0, 1), binding(&owner, 2, 3)]);
        assert!(app.story_active(&owner.actor(1, 0)));
        assert!(!app.story_active(&owner.actor(3, 0)));
        assert_eq!(app.seats.len(), 2);
    }

    #[test]
    fn image_refresh_applies_and_clears_cache() {
        let owner = owner();
        let mut app = presentation(&owner, &[("localization/loc_english.txt", "")]);
        app.receive(&[event(RereleasePresentationEventKind::Rerelease(
            Q2RereleaseEvent::Sky {
                name: "sky".to_string(),
                rotation: 1.0,
                auto_rotate: false,
                axis: vec3(0.0, 0.0, 1.0),
            },
        ))]);
        app.prepare().expect("prepare");
        let before: Vec<u32> = app
            .view(&owner.actor(1, 0), 0.0)
            .q2_sky
            .expect("sky")
            .images
            .iter()
            .map(|image| image.ordinal)
            .collect();
        assert_eq!(before, vec![0, 1, 2, 3, 4, 5]);
        let refresh = app.prepare_image_refresh();
        refresh.apply(&mut app);
        let after: Vec<u32> = app
            .view(&owner.actor(1, 0), 0.0)
            .q2_sky
            .expect("sky")
            .images
            .iter()
            .map(|image| image.ordinal)
            .collect();
        assert_eq!(after, vec![6, 7, 8, 9, 10, 11]);
        assert!(app.skies.is_empty());
    }

    #[test]
    fn draw_story_centers_and_validates_seat() {
        let owner = owner();
        let mut app = presentation(&owner, &[("localization/loc_english.txt", "STORY = \"Hi\"\n")]);
        app.receive(&[event(RereleasePresentationEventKind::Rerelease(
            Q2RereleaseEvent::Story {
                text: "$STORY".to_string(),
            },
        ))]);
        app.prepare().expect("prepare");
        let font = TextFontSelection::Classic {
            classic: classic_charset(0, 128, 128, "test", false).expect("font"),
            unicode: None,
        };
        let actor = owner.actor(1, 0);
        let text = SeatTextPresentation::new(owner.seat(0), &font);
        let mut sink = TextCommandSink::new(
            owner.seat(0),
            DrawRect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
        );
        {
            let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Pixels);
            app.draw_story(&actor, &mut draw, &text, 2.0).expect("draw");
        }
        assert!(!sink.commands.is_empty());
        // Another seat's font is rejected.
        let other = SeatTextPresentation::new(owner.seat(1), &font);
        let mut sink = TextCommandSink::new(
            owner.seat(0),
            DrawRect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
        );
        {
            let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Pixels);
            assert!(matches!(
                app.draw_story(&actor, &mut draw, &other, 2.0),
                Err(RereleasePresentationError::StorySeatMismatch)
            ));
        }
        // Unknown actors draw nothing.
        let mut sink = TextCommandSink::new(
            owner.seat(1),
            DrawRect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
        );
        {
            let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Pixels);
            app.draw_story(
                &owner.actor(42, 0),
                &mut draw,
                &SeatTextPresentation::new(owner.seat(1), &font),
                2.0,
            )
            .expect("empty");
        }
        assert!(sink.commands.is_empty());
    }
}
