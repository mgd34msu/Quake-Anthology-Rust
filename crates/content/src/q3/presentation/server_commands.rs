//! Quake III presentation: server commands.
//!
//! Donor provenance: `src/content/q3/presentation/server-commands.ts`.

use crate::q3anim::PlayerGender;
use qa_core::cmd::ascii_fold;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::*;
use crate::q3::presentation::client_info::ClientInfo;
use crate::q3::presentation::retail_snapshot::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Server commands (server-commands.ts)
// ---------------------------------------------------------------------------

/// Truncate at NUL and require source byte characters (`bytes`).
pub fn source_bytes(input: &str) -> PresentResult<String> {
    let end = input.find('\0').unwrap_or(input.len());
    let text = &input[..end];
    if text.chars().any(|c| c as u32 > 255) {
        return Err(range_msg("Cgame text requires source byte characters"));
    }
    Ok(text.to_owned())
}

/// Cvar read snapshot (`CvarSnapshot`, reduced to read fields).
#[derive(Debug, Clone, PartialEq)]
pub struct CvarSnapshot {
    /// Name.
    pub name: String,
    /// Value.
    pub value: String,
    /// Numeric value.
    pub numeric_value: f32,
    /// Integer value.
    pub integer_value: i32,
}

/// Server-command cvars (`ClientServerCommandCvar`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientServerCommandCvar {
    /// Team chat height.
    CgTeamChatHeight,
    /// Team chat time.
    CgTeamChatTime,
    /// Team chats only.
    CgTeamChatsOnly,
    /// Show miss.
    CgShowmiss,
    /// Single-player active.
    UiSinglePlayerActive,
    /// Record SP demo.
    UiRecordSPDemo,
    /// Record SP demo name.
    UiRecordSPDemoName,
    /// Build script.
    ComBuildScript,
    /// No voice chats.
    CgNoVoiceChats,
    /// No voice text.
    CgNoVoiceText,
    /// No taunt.
    CgNoTaunt,
}

/// Server-command sounds (`ClientServerCommandSound`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientServerCommandSound {
    /// Prepare count.
    CountPrepare,
    /// Team prepare count.
    CountPrepareTeam,
    /// Fight count.
    CountFight,
    /// Talk.
    Talk,
    /// Vote now.
    VoteNow,
    /// Vote passed.
    VotePassed,
    /// Vote failed.
    VoteFailed,
}

/// Server-command services (`ClientServerCommandHost`).
pub trait ClientServerCommandHost {
    /// Client info by slot.
    fn client_info<'a>(&self, static_state: &'a ClientGameStaticState, index: usize) -> PresentResult<&'a ClientInfo>;
    /// Install client info from a configstring.
    fn new_client_info(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        index: usize,
        configstring: &str,
    ) -> PresentResult<()>;
    /// Load deferred players.
    fn load_deferred_players(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()>;
    /// Reset client slots.
    fn reset_clients(&mut self, static_state: &mut ClientGameStaticState);
    /// Register a model.
    fn register_model(&mut self, path: &str) -> PresentResult<SceneModel>;
    /// Whether an asset exists.
    fn assets_has(&self, path: &str) -> bool;
    /// Read an asset.
    fn assets_read(&mut self, path: &str) -> PresentResult<Vec<u8>>;
    /// Read an asset synchronously.
    fn assets_read_sync(&self, path: &str) -> PresentResult<Vec<u8>>;
    /// Get a server command by sequence.
    fn get_server_command(&mut self, sequence: i32) -> PresentResult<Option<Vec<String>>>;
    /// Refresh the owned configstring snapshot.
    fn refresh_game_state(&mut self);
    /// Configstring by index.
    fn config_string(&self, index: i32) -> PresentResult<String>;
    /// Read a VM cvar.
    fn read_vm_cvar(&self, name: ClientServerCommandCvar) -> CvarSnapshot;
    /// Set a cvar.
    fn set_cvar(&mut self, name: &str, value: &str);
    /// Print.
    fn print(&mut self, text: &str);
    /// Center print.
    fn center_print(&mut self, text: &str, y: i32, char_width: i32);
    /// Send a console command.
    fn send_console_command(&mut self, text: &str);
    /// Named sound.
    fn sound(&self, name: ClientServerCommandSound) -> Option<PcmSound>;
    /// Register a sound.
    fn register_sound(&mut self, path: &str, compressed: bool) -> PresentResult<Option<PcmSound>>;
    /// Start a local sound.
    fn start_local_sound(&mut self, sound: Option<PcmSound>, channel: i32);
    /// Start the background track.
    fn start_background_track(&mut self, intro: &str, loop_track: &str) -> PresentResult<()>;
    /// Remap a shader.
    fn remap_shader(&mut self, original: &str, replacement: &str, time_offset: &str) -> PresentResult<()>;
    /// Clear local entities.
    fn clear_local_entities(&mut self);
    /// Clear marks.
    fn clear_marks(&mut self);
    /// Clear particles.
    fn clear_particles(&mut self) -> PresentResult<()>;
    /// Clear looping sounds.
    fn clear_looping_sounds(&mut self, kill_all: bool);
    /// Set the mission score selection.
    fn set_score_selection(&mut self);
    /// Show the response head.
    fn show_response_head(&mut self) -> PresentResult<()>;
    /// Remaining memory.
    fn memory_remaining(&self) -> i64;
    /// Random float (`random`).
    fn random_float(&mut self) -> f32;
}

/// Voice chat entry.
#[derive(Debug, Clone)]
pub(crate) struct VoiceChat {
    id: String,
    num_sounds: usize,
    sounds: [Option<PcmSound>; 64],
    chats: [String; 64],
}

pub(crate) fn empty_voice_chat() -> VoiceChat {
    VoiceChat {
        id: String::new(),
        num_sounds: 0,
        sounds: std::array::from_fn(|_| None),
        chats: std::array::from_fn(|_| String::new()),
    }
}

/// Voice chat list.
#[derive(Debug, Clone)]
pub(crate) struct VoiceChatList {
    name: String,
    gender: PlayerGender,
    num_voice_chats: usize,
    voice_chats: Vec<VoiceChat>,
}

pub(crate) fn empty_voice_list() -> VoiceChatList {
    VoiceChatList {
        name: String::new(),
        gender: PlayerGender::Male,
        num_voice_chats: 0,
        voice_chats: vec![empty_voice_chat(); 64],
    }
}

/// Head-model voice binding.
#[derive(Debug, Clone)]
pub(crate) struct HeadVoice {
    headmodel: String,
    voice_chat_num: usize,
}

/// Buffered voice chat.
#[derive(Debug, Clone)]
pub(crate) struct BufferedVoice {
    client_num: i32,
    snd: Option<PcmSound>,
    voice_only: bool,
    cmd: String,
    message: String,
}

pub(crate) fn empty_buffered_voice() -> BufferedVoice {
    BufferedVoice {
        client_num: 0,
        snd: None,
        voice_only: false,
        cmd: String::new(),
        message: String::new(),
    }
}

pub(crate) fn server_argv(values: &[String], index: usize) -> PresentResult<String> {
    let Some(value) = values.get(index) else {
        return Ok(String::new());
    };
    Ok(source_bytes(value)?.chars().take(1023).collect())
}

pub(crate) fn server_integer(values: &[String], index: usize) -> PresentResult<i32> {
    game_atoi(&server_argv(values, index)?)
}

pub(crate) fn server_command_copy(values: &[String]) -> PresentResult<Vec<String>> {
    if values.len() > 1024 {
        return Err(range_msg("Server command exceeds MAX_STRING_TOKENS"));
    }
    values
        .iter()
        .map(|value| Ok(source_bytes(value)?.chars().take(1023).collect()))
        .collect()
}

/// `COM_ParseExt` byte tokenizer (`SourceByteTokenizer`).
#[derive(Debug, Clone)]
pub struct SourceByteTokenizer {
    text: Vec<u8>,
    offset: usize,
}

impl SourceByteTokenizer {
    /// New tokenizer over validated source bytes.
    pub fn new(input: &str) -> PresentResult<Self> {
        Ok(Self {
            text: source_bytes(input)?.into_bytes(),
            offset: 0,
        })
    }

    fn byte(&self) -> i32 {
        if self.offset >= self.text.len() {
            return 0;
        }
        i32::from(self.text[self.offset] as i8)
    }

    fn starts_with(&self, token: &[u8]) -> bool {
        self.text.get(self.offset..).is_some_and(|rest| rest.starts_with(token))
    }

    /// Next token.
    pub fn next(&mut self, allow_line_breaks: bool) -> PresentResult<String> {
        let mut newline = false;
        loop {
            while self.byte() <= 32 {
                if self.byte() == 0 {
                    return Ok(String::new());
                }
                if self.byte() == 10 {
                    newline = true;
                }
                self.offset += 1;
            }
            if newline && !allow_line_breaks {
                return Ok(String::new());
            }
            if self.starts_with(b"//") {
                self.offset += 2;
                while self.byte() != 0 && self.byte() != 10 {
                    self.offset += 1;
                }
            } else if self.starts_with(b"/*") {
                self.offset += 2;
                while self.byte() != 0 && !self.starts_with(b"*/") {
                    self.offset += 1;
                }
                if self.byte() != 0 {
                    self.offset += 2;
                }
            } else {
                break;
            }
        }
        if self.byte() == 34 {
            let start = self.offset + 1;
            self.offset = start;
            while self.byte() != 0 && self.byte() != 34 {
                self.offset += 1;
            }
            let value = String::from_utf8_lossy(&self.text[start..self.offset]).into_owned();
            if value.len() >= 1024 {
                return Err(range_msg("Quoted source token overflows MAX_TOKEN_CHARS"));
            }
            if self.byte() == 34 {
                self.offset += 1;
            }
            return Ok(value);
        }
        let start = self.offset;
        while self.byte() > 32 {
            self.offset += 1;
        }
        let value = String::from_utf8_lossy(&self.text[start..self.offset]).into_owned();
        Ok(if value.len() >= 1024 { String::new() } else { value })
    }
}

pub(crate) fn valid_game_type(value: i32) -> PresentResult<GameType> {
    GameType::from_i32(value)
        .filter(|game| *game != GameType::GtMaxGameType)
        .ok_or_else(|| range_msg(format!("Invalid server game type {value}")))
}

pub(crate) fn empty_score() -> ClientScore {
    ClientScore {
        team: Team::TeamFree as i32,
        ..ClientScore::default()
    }
}

pub(crate) fn order_task(command: &str) -> i32 {
    match ascii_fold(command).as_str() {
        "getflag" | "offense" => 1,
        "defend" | "defendflag" => 2,
        "patrol" => 3,
        "followme" => 4,
        "returnflag" => 5,
        "followflagcarrier" => 6,
        "camp" => 7,
        _ => -1,
    }
}

/// Server-command runtime (`ClientServerCommandRuntime`).
pub struct ClientServerCommandRuntime<H> {
    /// Host services.
    pub host: H,
    closed: bool,
    voice_lists: [VoiceChatList; 8],
    head_voices: Vec<HeadVoice>,
    voices: Vec<BufferedVoice>,
}

impl<H: ClientServerCommandHost> ClientServerCommandRuntime<H> {
    /// New runtime.
    #[must_use]
    pub fn new(host: H) -> Self {
        Self {
            host,
            closed: false,
            voice_lists: std::array::from_fn(|_| empty_voice_list()),
            head_voices: vec![
                HeadVoice {
                    headmodel: String::new(),
                    voice_chat_num: 0
                };
                64
            ],
            voices: vec![empty_buffered_voice(); 32],
        }
    }

    /// Dispose the runtime.
    pub fn dispose(&mut self, static_state: &mut ClientGameStaticState) {
        self.closed = true;
        self.host.reset_clients(static_state);
    }

    fn open(&self) -> PresentResult<()> {
        if self.closed {
            return Err(state_msg("Cgame command runtime is closed"));
        }
        Ok(())
    }

    fn fail_closed(
        &mut self,
        static_state: &mut ClientGameStaticState,
        result: PresentResult<()>,
    ) -> PresentResult<()> {
        if result.is_err() && !self.closed {
            self.dispose(static_state);
        }
        result
    }

    fn check_products(&self, state: &ClientGameState, static_state: &ClientGameStaticState) -> PresentResult<()> {
        if state.product != static_state.product {
            return Err(state_msg("Cgame state products differ"));
        }
        Ok(())
    }

    fn cvar(&self, name: ClientServerCommandCvar) -> CvarSnapshot {
        self.host.read_vm_cvar(name)
    }

    fn config(&self, index: i32) -> PresentResult<String> {
        if !(0..1024).contains(&index) {
            return Err(range_msg("CG_ConfigString: bad index"));
        }
        let value = source_bytes(&self.host.config_string(index)?)?;
        if value.len() >= 16000 {
            return Err(range_msg("Configstring exceeds MAX_GAMESTATE_CHARS"));
        }
        Ok(value)
    }

    fn local_sound(&mut self, name: ClientServerCommandSound, channel: i32) {
        let sound = self.host.sound(name);
        self.host.start_local_sound(sound, channel);
    }

    /// Execute new server commands through a sequence.
    pub fn execute_new_server_commands(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        latest_sequence: i32,
    ) -> PresentResult<()> {
        if latest_sequence < 0 {
            self.fail_closed(static_state, Err(range_msg("Invalid reliable command sequence")))?;
            return Ok(());
        }
        self.open()?;
        let mut result = Ok(());
        while static_state.server_command_sequence < latest_sequence {
            static_state.server_command_sequence = static_state.server_command_sequence.wrapping_add(1);
            let sequence = static_state.server_command_sequence;
            let command = self.host.get_server_command(sequence);
            let command = match command {
                Ok(command) => command,
                Err(error) => {
                    result = Err(error);
                    break;
                }
            };
            if self.closed {
                result = Err(state_msg("Cgame command runtime is closed"));
                break;
            }
            if let Some(command) = command {
                let owned = match server_command_copy(&command) {
                    Ok(owned) => owned,
                    Err(error) => {
                        result = Err(error);
                        break;
                    }
                };
                if let Err(error) = self.dispatch(state, static_state, &owned) {
                    result = Err(error);
                    break;
                }
            }
        }
        self.fail_closed(static_state, result)
    }

    /// Execute one command.
    pub fn execute_command(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        command: &[String],
    ) -> PresentResult<()> {
        let owned = server_command_copy(command)?;
        self.open()?;
        let result = self.dispatch(state, static_state, &owned);
        self.fail_closed(static_state, result)
    }

    /// Parse the server info configstring.
    pub fn parse_server_info(&mut self, static_state: &mut ClientGameStaticState) -> PresentResult<()> {
        self.open()?;
        let info = self.config(0)?;
        let value = |key: &str| info_value_for_key(&info, key, 8192).unwrap_or_default();
        static_state.game_type = valid_game_type(game_atoi(&value("g_gametype"))?)?;
        self.host
            .set_cvar("g_gametype", &(static_state.game_type as i32).to_string());
        static_state.dm_flags = game_atoi(&value("dmflags"))?;
        static_state.team_flags = game_atoi(&value("teamflags"))?;
        static_state.fraglimit = game_atoi(&value("fraglimit"))?;
        static_state.capturelimit = game_atoi(&value("capturelimit"))?;
        static_state.timelimit = game_atoi(&value("timelimit"))?;
        static_state.maxclients = game_atoi(&value("sv_maxclients"))?;
        static_state.mapname = format!("maps/{}.bsp", value("mapname")).chars().take(63).collect();
        static_state.red_team = value("g_redTeam").chars().take(63).collect();
        let red = static_state.red_team.clone();
        self.host.set_cvar("g_redTeam", &red);
        static_state.blue_team = value("g_blueTeam").chars().take(63).collect();
        let blue = static_state.blue_team.clone();
        self.host.set_cvar("g_blueTeam", &blue);
        Ok(())
    }

    fn flag_status(&mut self, state: &ClientGameState, static_state: &mut ClientGameStaticState) -> PresentResult<()> {
        let value = self.config(23)?;
        let bytes: Vec<char> = value.chars().collect();
        if static_state.game_type == GameType::GtCtf {
            if bytes.is_empty() {
                return Err(range_msg("CTF flag status leaves source bytes uninitialized"));
            }
            static_state.redflag = i32::from(bytes[0] as u8).wrapping_sub(48);
            static_state.blueflag = i32::from(bytes.get(1).copied().unwrap_or('\0') as u8).wrapping_sub(48);
        } else if state.product == Product::Missionpack && static_state.game_type == GameType::Gt1fctf {
            static_state.flag_status = i32::from(bytes.first().copied().unwrap_or('\0') as u8).wrapping_sub(48);
        }
        Ok(())
    }

    /// Set config-driven values.
    pub fn set_config_values(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()> {
        self.open()?;
        self.check_products(state, static_state)?;
        static_state.scores1 = game_atoi(&self.config(6)?)?;
        static_state.scores2 = game_atoi(&self.config(7)?)?;
        static_state.level_start_time = game_atoi(&self.config(21)?)?;
        self.flag_status(state, static_state)?;
        state.warmup = game_atoi(&self.config(5)?)?;
        Ok(())
    }

    fn parse_warmup(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()> {
        let warmup = game_atoi(&self.config(5)?)?;
        state.warmup_count = -1;
        if warmup > 0 && state.warmup <= 0 {
            let team_sound = state.product == Product::Missionpack
                && (static_state.game_type as i32) >= (GameType::GtCtf as i32)
                && (static_state.game_type as i32) <= (GameType::GtHarvester as i32);
            self.local_sound(
                if team_sound {
                    ClientServerCommandSound::CountPrepareTeam
                } else {
                    ClientServerCommandSound::CountPrepare
                },
                7,
            );
        }
        state.warmup = warmup;
        Ok(())
    }

    fn parse_scores(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        command: &[String],
    ) -> PresentResult<()> {
        state.num_scores = server_integer(command, 1)?.min(MAX_CLIENTS as i32);
        state.team_scores[0] = server_integer(command, 2)?;
        state.team_scores[1] = server_integer(command, 3)?;
        for slot in state.scores.iter_mut() {
            *slot = empty_score();
        }
        for i in 0..state.num_scores {
            let base = i.wrapping_mul(14);
            let number = server_integer(command, base as usize + 4)?;
            let client = if number < 0 || number >= MAX_CLIENTS as i32 {
                0
            } else {
                number
            };
            let score = ClientScore {
                client,
                score: server_integer(command, base as usize + 5)?,
                ping: server_integer(command, base as usize + 6)?,
                time: server_integer(command, base as usize + 7)?,
                score_flags: server_integer(command, base as usize + 8)?,
                accuracy: server_integer(command, base as usize + 10)?,
                impressive_count: server_integer(command, base as usize + 11)?,
                excellent_count: server_integer(command, base as usize + 12)?,
                guantlet_count: server_integer(command, base as usize + 13)?,
                defend_count: server_integer(command, base as usize + 14)?,
                assist_count: server_integer(command, base as usize + 15)?,
                perfect: server_integer(command, base as usize + 16)?,
                captures: server_integer(command, base as usize + 17)?,
                team: self.host.client_info(static_state, client as usize)?.team as i32,
            };
            state.scores[i as usize] = score;
            let info = at_mut(&mut static_state.client_info, client as usize, "Source array index")?;
            info.score = score.score;
            info.powerups = server_integer(command, base as usize + 9)?;
        }
        if state.product == Product::Missionpack {
            self.host.set_score_selection();
        }
        Ok(())
    }

    fn parse_team_info(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        command: &[String],
    ) -> PresentResult<()> {
        let count = server_integer(command, 1)?;
        if count > 8 {
            return Err(range_msg("Team overlay exceeds TEAM_MAXOVERLAY"));
        }
        state.num_sorted_team_players = count;
        for i in 0..count {
            let client = server_integer(command, i as usize * 6 + 2)?;
            if client < 0 || client >= MAX_CLIENTS as i32 {
                return Err(range_msg(format!("Source array index {client} outside 64")));
            }
            let info = at_mut(&mut static_state.client_info, client as usize, "Source array index")?;
            state.sorted_team_players[i as usize] = client;
            info.location = server_integer(command, i as usize * 6 + 3)?;
            info.health = server_integer(command, i as usize * 6 + 4)?;
            info.armor = server_integer(command, i as usize * 6 + 5)?;
            info.cur_weapon = server_integer(command, i as usize * 6 + 6)?;
            info.powerups = server_integer(command, i as usize * 6 + 7)?;
        }
        Ok(())
    }

    /// Apply shader-state changes.
    pub fn shader_state_changed(&mut self) -> PresentResult<()> {
        self.open()?;
        let value = self.config(24)?;
        let mut offset = 0;
        while offset < value.len() {
            let Some(equals) = value[offset..].find('=').map(|index| offset + index) else {
                break;
            };
            let Some(colon) = value[equals + 1..].find(':').map(|index| equals + 1 + index) else {
                break;
            };
            let Some(end) = value[colon + 1..].find('@').map(|index| colon + 1 + index) else {
                break;
            };
            let original = &value[offset..equals];
            let replacement = &value[equals + 1..colon];
            let time = &value[colon + 1..end];
            if original.len() >= 64 || replacement.len() >= 64 || time.len() >= 16 {
                return Err(range_msg("Shader remap exceeds source scratch buffers"));
            }
            self.host.remap_shader(original, replacement, time)?;
            self.open()?;
            offset = end + 1;
        }
        Ok(())
    }

    /// Start the configstring music.
    pub fn start_music(&mut self) -> PresentResult<()> {
        self.open()?;
        let mut tokenizer = SourceByteTokenizer::new(&self.config(2)?)?;
        let first = tokenizer.next(true)?;
        let second = tokenizer.next(true)?;
        let first: String = first.chars().take(63).collect();
        let second: String = second.chars().take(63).collect();
        self.host.start_background_track(&first, &second)?;
        self.open()?;
        Ok(())
    }

    /// Build the spectator string.
    pub fn build_spectator_string(
        &mut self,
        state: &mut ClientGameState,
        static_state: &ClientGameStaticState,
    ) -> PresentResult<()> {
        self.open()?;
        state.spectator_list = String::new();
        for i in 0..MAX_CLIENTS {
            let info = self.host.client_info(static_state, i)?;
            if info.info_valid && info.team == Team::TeamSpectator {
                state.spectator_list = format!("{}{}     ", state.spectator_list, info.name)
                    .chars()
                    .take(1023)
                    .collect();
            }
        }
        if state.spectator_list.len() != state.spectator_len {
            state.spectator_len = state.spectator_list.len();
            state.spectator_width = -1;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn config_modified(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        command: &[String],
    ) -> PresentResult<()> {
        let index = server_integer(command, 1)?;
        self.host.refresh_game_state();
        let value = self.config(index)?;
        match index {
            0 => {
                self.parse_server_info(static_state)?;
                return Ok(());
            }
            2 => {
                self.start_music()?;
                return Ok(());
            }
            5 => {
                self.parse_warmup(state, static_state)?;
                return Ok(());
            }
            6 => {
                static_state.scores1 = game_atoi(&value)?;
                return Ok(());
            }
            7 => {
                static_state.scores2 = game_atoi(&value)?;
                return Ok(());
            }
            8 => {
                static_state.vote_time = game_atoi(&value)?;
                static_state.vote_modified = true;
                return Ok(());
            }
            9 => {
                static_state.vote_string = value.chars().take(1023).collect();
                if state.product == Product::Missionpack {
                    self.local_sound(ClientServerCommandSound::VoteNow, 7);
                }
                return Ok(());
            }
            10 => {
                static_state.vote_yes = game_atoi(&value)?;
                static_state.vote_modified = true;
                return Ok(());
            }
            11 => {
                static_state.vote_no = game_atoi(&value)?;
                static_state.vote_modified = true;
                return Ok(());
            }
            21 => {
                static_state.level_start_time = game_atoi(&value)?;
                return Ok(());
            }
            22 => {
                state.intermission_started = game_atoi(&value)? != 0;
                return Ok(());
            }
            23 => {
                self.flag_status(state, static_state)?;
                return Ok(());
            }
            24 => {
                self.shader_state_changed()?;
                return Ok(());
            }
            _ => {}
        }
        if (12..20).contains(&index) {
            let slot = (index % 2) as usize;
            if index < 14 {
                static_state.team_vote_time[slot] = game_atoi(&value)?;
                static_state.team_vote_modified[slot] = true;
            } else if index < 16 {
                if value.len() >= 1024 {
                    return Err(range_msg("Team vote string exceeds source row buffer"));
                }
                static_state.team_vote_string[slot] = value;
                if state.product == Product::Missionpack {
                    self.local_sound(ClientServerCommandSound::VoteNow, 7);
                }
            } else if index < 18 {
                static_state.team_vote_yes[slot] = game_atoi(&value)?;
                static_state.team_vote_modified[slot] = true;
            } else {
                static_state.team_vote_no[slot] = game_atoi(&value)?;
                static_state.team_vote_modified[slot] = true;
            }
        } else if (32..288).contains(&index) {
            let model = self.host.register_model(&value)?;
            self.open()?;
            static_state.game_models[(index - 32) as usize] = model;
        } else if (288..544).contains(&index) {
            if !value.starts_with('*') {
                let sound = self.host.register_sound(&value, false)?;
                self.open()?;
                static_state.game_sounds[(index - 288) as usize] = sound;
            }
        } else if (544..608).contains(&index) {
            self.host
                .new_client_info(state, static_state, (index - 544) as usize, &value)?;
            self.open()?;
            self.build_spectator_string(state, static_state)?;
        }
        Ok(())
    }

    /// Add wrapped lines to team chat.
    pub fn add_to_team_chat(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        input: &str,
    ) -> PresentResult<()> {
        self.open()?;
        let height = self
            .cvar(ClientServerCommandCvar::CgTeamChatHeight)
            .integer_value
            .min(8);
        if height <= 0 || self.cvar(ClientServerCommandCvar::CgTeamChatTime).integer_value <= 0 {
            static_state.team_chat_pos = 0;
            static_state.team_last_chat_pos = 0;
            return Ok(());
        }
        let text = source_bytes(input)?;
        let chars: Vec<char> = text.chars().collect();
        let mut offset = 0;
        let mut line = String::new();
        let mut visible = 0;
        let mut last_space: Option<usize> = None;
        let mut color = '7';
        let publish = |line: &mut String,
                       static_state: &mut ClientGameStaticState,
                       state: &ClientGameState|
         -> PresentResult<()> {
            if line.len() > 240 {
                return Err(range_msg("Team chat exceeds source color-expanded line buffer"));
            }
            let slot = static_state.team_chat_pos.rem_euclid(height) as usize;
            static_state.team_chat_msgs[slot] = std::mem::take(line);
            static_state.team_chat_msg_times[slot] = state.time;
            static_state.team_chat_pos = static_state.team_chat_pos.wrapping_add(1);
            Ok(())
        };
        while offset < chars.len() {
            if visible > 79 {
                if let Some(last) = last_space {
                    let cut = line.len() - last;
                    offset = offset.saturating_sub(cut).saturating_add(1);
                    line.truncate(last);
                }
                publish(&mut line, static_state, state)?;
                line = format!("^{color}");
                visible = 0;
                last_space = None;
            }
            let Some(character) = chars.get(offset).copied() else {
                break;
            };
            let next = chars.get(offset + 1).copied();
            if character == '^' && !matches!(next, None | Some('^')) {
                let next = next.expect("checked caret escape");
                line.push(character);
                line.push(next);
                color = next;
                offset += 2;
                continue;
            }
            if character == ' ' {
                last_space = Some(line.len());
            }
            line.push(character);
            offset += 1;
            visible += 1;
        }
        publish(&mut line, static_state, state)?;
        if static_state.team_chat_pos.wrapping_sub(static_state.team_last_chat_pos) > height {
            static_state.team_last_chat_pos = static_state.team_chat_pos.wrapping_sub(height);
        }
        Ok(())
    }

    fn map_restart(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()> {
        if self.cvar(ClientServerCommandCvar::CgShowmiss).integer_value != 0 {
            self.host.print("CG_MapRestart\n");
        }
        self.host.clear_local_entities();
        self.host.clear_marks();
        self.host.clear_particles()?;
        state.fraglimit_warnings = 0;
        state.timelimit_warnings = 0;
        state.intermission_started = false;
        static_state.vote_time = 0;
        state.map_restart = true;
        self.start_music()?;
        self.host.clear_looping_sounds(true);
        if state.warmup == 0 {
            self.local_sound(ClientServerCommandSound::CountFight, 7);
            self.host.center_print("FIGHT!", 120, 64);
        }
        if state.product == Product::Missionpack
            && self.cvar(ClientServerCommandCvar::UiSinglePlayerActive).integer_value != 0
        {
            self.host.set_cvar("ui_matchStartTime", &state.time.to_string());
            let demo = source_bytes(&self.cvar(ClientServerCommandCvar::UiRecordSPDemoName).value)?;
            if self.cvar(ClientServerCommandCvar::UiRecordSPDemo).integer_value != 0 && !demo.is_empty() {
                self.host
                    .send_console_command(&format!("set g_synchronousclients 1 ; record {demo} \n"));
            }
        }
        self.host.set_cvar("cg_thirdPerson", "0");
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn dispatch(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        command: &[String],
    ) -> PresentResult<()> {
        let name = server_argv(command, 0)?;
        match name.as_str() {
            "" => return Ok(()),
            "cp" => {
                let y = match state.product {
                    Product::Baseq3 => 143,
                    Product::Missionpack => 144,
                };
                self.host.center_print(&server_argv(command, 1)?, y, 16);
                return Ok(());
            }
            "cs" => {
                self.config_modified(state, static_state, command)?;
                return Ok(());
            }
            "print" => {
                let value = server_argv(command, 1)?;
                self.host.print(&value);
                if state.product == Product::Missionpack {
                    let text = ascii_fold(&value);
                    if text.starts_with("vote failed") || text.starts_with("team vote failed") {
                        self.local_sound(ClientServerCommandSound::VoteFailed, 7);
                    } else if text.starts_with("vote passed") || text.starts_with("team vote passed") {
                        self.local_sound(ClientServerCommandSound::VotePassed, 7);
                    }
                }
                return Ok(());
            }
            "chat" | "tchat" => {
                if name == "chat" && self.cvar(ClientServerCommandCvar::CgTeamChatsOnly).integer_value != 0 {
                    return Ok(());
                }
                self.local_sound(ClientServerCommandSound::Talk, 6);
                let value: String = server_argv(command, 1)?.chars().take(149).collect();
                let value = value.replace('\u{19}', "");
                if name == "tchat" {
                    self.add_to_team_chat(state, static_state, &value)?;
                }
                self.host.print(&format!("{value}\n"));
                return Ok(());
            }
            "vchat" | "vtchat" | "vtell" => {
                if state.product == Product::Missionpack {
                    let id = server_argv(command, 4)?;
                    if self.cvar(ClientServerCommandCvar::CgNoTaunt).integer_value != 0
                        && ["kill_insult", "taunt", "death_insult", "kill_gauntlet", "praise"].contains(&id.as_str())
                    {
                        return Ok(());
                    }
                    let mode = if name == "vchat" {
                        0
                    } else if name == "vtchat" {
                        1
                    } else {
                        2
                    };
                    self.voice_chat_local(
                        state,
                        static_state,
                        mode,
                        server_integer(command, 1)? != 0,
                        server_integer(command, 2)?,
                        server_integer(command, 3)?,
                        &id,
                    )?;
                }
                return Ok(());
            }
            "scores" => {
                self.parse_scores(state, static_state, command)?;
                return Ok(());
            }
            "tinfo" => {
                self.parse_team_info(state, static_state, command)?;
                return Ok(());
            }
            "map_restart" => {
                self.map_restart(state, static_state)?;
                return Ok(());
            }
            "loaddefered" => {
                self.host.load_deferred_players(state, static_state)?;
                self.open()?;
                return Ok(());
            }
            "clientLevelShot" => {
                state.level_shot = true;
                return Ok(());
            }
            _ => {}
        }
        let mut remaining = name;
        if ascii_fold(&remaining) == "remapshader" && command.len() == 4 {
            remaining = server_argv(command, 3)?;
            self.host.remap_shader(&remaining, &remaining, &remaining)?;
            self.open()?;
            if remaining == "loaddefered" {
                self.host.load_deferred_players(state, static_state)?;
                self.open()?;
                return Ok(());
            }
            if remaining == "clientLevelShot" {
                state.level_shot = true;
                return Ok(());
            }
        }
        self.host.print(&format!("Unknown client game command: {remaining}\n"));
        Ok(())
    }

    fn voice_file(&mut self, filename: &str, missing_warning: bool) -> PresentResult<Option<String>> {
        self.open()?;
        if !self.host.assets_has(filename) {
            if missing_warning {
                self.host.print(&format!("^1voice chat file not found: {filename}\n"));
            }
            return Ok(None);
        }
        let data = self.host.assets_read(filename)?;
        self.open()?;
        if data.len() >= 16384 {
            self.host.print(&format!(
                "^1voice chat file too large: {filename} is {}, max allowed is 16384",
                data.len()
            ));
            return Ok(None);
        }
        Ok(Some(data.iter().map(|byte| *byte as char).collect()))
    }

    /// Parse a voice-chat script into a list.
    pub fn parse_voice_chats(
        &mut self,
        filename: &str,
        list_index: usize,
        maximum_chats: usize,
    ) -> PresentResult<bool> {
        self.open()?;
        if !(1..=64).contains(&maximum_chats) {
            return Err(range_msg("Invalid voice chat count"));
        }
        if list_index >= self.voice_lists.len() {
            return Err(range_msg(format!("Source array index {list_index} outside 8")));
        }
        let compressed = self.cvar(ClientServerCommandCvar::ComBuildScript).integer_value == 0;
        let Some(text) = self.voice_file(filename, true)? else {
            return Ok(false);
        };
        let list = &mut self.voice_lists[list_index];
        list.name = source_bytes(filename)?.chars().take(63).collect();
        for chat in list.voice_chats.iter_mut().take(maximum_chats) {
            chat.id = String::new();
        }
        let mut tokenizer = SourceByteTokenizer::new(&text)?;
        let gender = ascii_fold(&tokenizer.next(true)?);
        if gender.is_empty() {
            return Ok(true);
        }
        if gender != "male" && gender != "female" && gender != "neuter" {
            self.host
                .print(&format!("^1expected gender not found in voice chat file: {filename}\n"));
            return Ok(false);
        }
        {
            let list = &mut self.voice_lists[list_index];
            list.gender = match gender.as_str() {
                "female" => PlayerGender::Female,
                "neuter" => PlayerGender::Neuter,
                _ => PlayerGender::Male,
            };
            list.num_voice_chats = 0;
        }
        loop {
            let id = tokenizer.next(true)?;
            if id.is_empty() {
                return Ok(true);
            }
            {
                let list = &mut self.voice_lists[list_index];
                if list.num_voice_chats >= list.voice_chats.len() {
                    return Err(range_msg("Voice chat list overflow"));
                }
                let chat_index = list.num_voice_chats;
                list.voice_chats[chat_index].id = id.chars().take(63).collect();
            }
            let brace = tokenizer.next(true)?;
            if brace != "{" {
                self.host
                    .print(&format!("^1expected {{ found {brace} in voice chat file: {filename}\n"));
                return Ok(false);
            }
            {
                let list = &mut self.voice_lists[list_index];
                let chat_index = list.num_voice_chats;
                list.voice_chats[chat_index].num_sounds = 0;
            }
            loop {
                let path = tokenizer.next(true)?;
                if path.is_empty() {
                    return Ok(true);
                }
                if path == "}" {
                    break;
                }
                let sound = self.host.register_sound(&path, compressed)?;
                self.open()?;
                {
                    let list = &mut self.voice_lists[list_index];
                    let chat_index = list.num_voice_chats;
                    let chat = &mut list.voice_chats[chat_index];
                    chat.sounds[chat.num_sounds] = sound;
                }
                let message = tokenizer.next(true)?;
                if message.is_empty() {
                    return Ok(true);
                }
                {
                    let list = &mut self.voice_lists[list_index];
                    let chat_index = list.num_voice_chats;
                    let chat = &mut list.voice_chats[chat_index];
                    chat.chats[chat.num_sounds] = message.chars().take(63).collect();
                    if sound.is_some() {
                        chat.num_sounds += 1;
                    }
                    if chat.num_sounds >= 64 {
                        break;
                    }
                }
            }
            let list = &mut self.voice_lists[list_index];
            list.num_voice_chats += 1;
            if list.num_voice_chats >= maximum_chats {
                return Ok(true);
            }
        }
    }

    /// Load the stock voice-chat scripts.
    pub fn load_voice_chats(&mut self) -> PresentResult<()> {
        self.open()?;
        let before = self.host.memory_remaining();
        for (index, file) in [
            "female1", "female2", "female3", "male1", "male2", "male3", "male4", "male5",
        ]
        .iter()
        .enumerate()
        {
            self.parse_voice_chats(&format!("scripts/{file}.voice"), index, 64)?;
        }
        let after = self.host.memory_remaining();
        self.host
            .print(&format!("voice chat memory size = {}\n", before - after));
        Ok(())
    }

    fn head_model_voice_chats(&mut self, filename: &str) -> PresentResult<i32> {
        if !self.host.assets_has(filename) {
            return Ok(-1);
        }
        let data = self.host.assets_read_sync(filename)?;
        if data.len() >= 16384 {
            self.host.print(&format!(
                "^1voice chat file too large: {filename} is {}, max allowed is 16384",
                data.len()
            ));
            return Ok(-1);
        }
        let text: String = data.iter().map(|byte| *byte as char).collect();
        let token = SourceByteTokenizer::new(&source_bytes(&text)?)?.next(true)?;
        if token.is_empty() {
            return Ok(-1);
        }
        Ok(self
            .voice_lists
            .iter()
            .position(|list| ascii_fold(&list.name) == ascii_fold(&token))
            .map_or(-1, |index| index as i32))
    }

    fn voice_list_for_client(&mut self, static_state: &ClientGameStaticState, client_num: i32) -> PresentResult<usize> {
        let clamped = if client_num < 0 || client_num >= MAX_CLIENTS as i32 {
            0
        } else {
            client_num
        };
        let info = self.host.client_info(static_state, clamped as usize)?.clone();
        let model = info
            .head_model_name
            .strip_prefix('*')
            .unwrap_or(&info.head_model_name)
            .to_owned();
        let mut head = String::new();
        for candidate in [format!("{model}/{}", info.head_skin_name), model.clone()] {
            head = candidate.chars().take(63).collect();
            if let Some(cached) = self
                .head_voices
                .iter()
                .find(|entry| ascii_fold(&entry.headmodel) == ascii_fold(&head))
            {
                return Ok(cached.voice_chat_num);
            }
            if let Some(free_index) = self.head_voices.iter().position(|entry| entry.headmodel.is_empty()) {
                let script: String = format!("scripts/{head}.vc").chars().take(63).collect();
                let index = self.head_model_voice_chats(&script)?;
                if index >= 0 {
                    self.head_voices[free_index].headmodel.clone_from(&head);
                    self.head_voices[free_index].voice_chat_num = index as usize;
                    return Ok(index as usize);
                }
            }
        }
        let genders: Vec<PlayerGender> = if info.gender == PlayerGender::Male {
            vec![PlayerGender::Male]
        } else {
            vec![info.gender, PlayerGender::Male]
        };
        for gender in genders {
            if let Some(index) = self
                .voice_lists
                .iter()
                .position(|list| !list.name.is_empty() && list.gender == gender)
            {
                if let Some(free) = self.head_voices.iter_mut().find(|entry| entry.headmodel.is_empty()) {
                    free.headmodel = head.clone();
                    free.voice_chat_num = index;
                }
                return Ok(index);
            }
        }
        if let Some(free) = self.head_voices.iter_mut().find(|entry| entry.headmodel.is_empty()) {
            free.headmodel = head.clone();
            free.voice_chat_num = 0;
        }
        Ok(0)
    }

    /// Queue a local voice chat.
    #[allow(clippy::too_many_arguments)]
    pub fn voice_chat_local(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        mode: i32,
        voice_only: bool,
        client_number: i32,
        color: i32,
        command: &str,
    ) -> PresentResult<()> {
        self.open()?;
        if state.product != Product::Missionpack || state.intermission_started {
            return Ok(());
        }
        let client_num = if client_number < 0 || client_number >= MAX_CLIENTS as i32 {
            0
        } else {
            client_number
        };
        let info = self.host.client_info(static_state, client_num as usize)?.clone();
        static_state.current_voice_client = client_num;
        let list_index = self.voice_list_for_client(static_state, client_num)?;
        let list = self.voice_lists[list_index].clone();
        let Some(chat) = list
            .voice_chats
            .iter()
            .take(list.num_voice_chats)
            .find(|entry| ascii_fold(&entry.id) == ascii_fold(command))
            .cloned()
        else {
            return Ok(());
        };
        let index = (self.host.random_float() * chat.num_sounds as f32).trunc() as usize;
        let sound = chat.sounds.get(index).copied().flatten();
        let message = chat.chats.get(index).cloned().unwrap_or_default();
        if mode != 1 && self.cvar(ClientServerCommandCvar::CgTeamChatsOnly).integer_value != 0 {
            return Ok(());
        }
        let name = if mode == 2 {
            format!("[{}]", info.name)
        } else if mode == 1 {
            format!("({})", info.name)
        } else {
            info.name.clone()
        };
        let message = source_bytes(&format!("{name}: ^{}{message}", char::from((color & 255) as u8)))?
            .chars()
            .take(149)
            .collect();
        let cmd = source_bytes(command)?.chars().take(149).collect();
        self.add_buffered_voice_chat(
            state,
            static_state,
            BufferedVoice {
                client_num,
                snd: sound,
                voice_only,
                cmd,
                message,
            },
        )
    }

    fn add_buffered_voice_chat(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        voice: BufferedVoice,
    ) -> PresentResult<()> {
        if state.intermission_started {
            return Ok(());
        }
        at(&self.voices, state.voice_chat_buffer_in, "Source array index")?;
        self.voices[state.voice_chat_buffer_in] = voice;
        state.voice_chat_buffer_in = (state.voice_chat_buffer_in + 1) % 32;
        if state.voice_chat_buffer_in == state.voice_chat_buffer_out {
            let voice = at(&self.voices, state.voice_chat_buffer_out, "Source array index")?.clone();
            self.play_voice_chat(state, static_state, &voice)?;
            state.voice_chat_buffer_out = state.voice_chat_buffer_out.wrapping_add(1);
        }
        Ok(())
    }

    fn play_voice_chat(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        voice: &BufferedVoice,
    ) -> PresentResult<()> {
        if state.intermission_started {
            return Ok(());
        }
        if self.cvar(ClientServerCommandCvar::CgNoVoiceChats).integer_value == 0 {
            self.host.start_local_sound(voice.snd, 3);
            let snapshot = state
                .snap
                .as_ref()
                .ok_or_else(|| state_msg("Voice playback requires the current snapshot"))?;
            if voice.client_num != snapshot.player_state.client_num {
                let order = order_task(&voice.cmd);
                if order > 0 {
                    static_state.accept_order_time = state.time.wrapping_add(5000);
                    static_state.accept_voice = voice.cmd.chars().take(31).collect();
                    static_state.accept_task = order;
                    static_state.accept_leader = voice.client_num;
                }
                self.host.show_response_head()?;
                self.open()?;
            }
        }
        if !voice.voice_only && self.cvar(ClientServerCommandCvar::CgNoVoiceText).integer_value == 0 {
            self.add_to_team_chat(state, static_state, &voice.message)?;
            self.host.print(&format!("{}\n", voice.message));
        }
        at_mut(&mut self.voices, state.voice_chat_buffer_out, "Source array index")?.snd = None;
        Ok(())
    }

    /// Play buffered voice chats.
    pub fn play_buffered_voice_chats(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()> {
        self.open()?;
        if state.product != Product::Missionpack || state.voice_chat_time >= state.time {
            return Ok(());
        }
        if state.voice_chat_buffer_out != state.voice_chat_buffer_in
            && at(&self.voices, state.voice_chat_buffer_out, "Source array index")?
                .snd
                .is_some()
        {
            let voice = at(&self.voices, state.voice_chat_buffer_out, "Source array index")?.clone();
            self.play_voice_chat(state, static_state, &voice)?;
            state.voice_chat_buffer_out = (state.voice_chat_buffer_out + 1) % 32;
            state.voice_chat_time = state.time.wrapping_add(1000);
        }
        Ok(())
    }
}
