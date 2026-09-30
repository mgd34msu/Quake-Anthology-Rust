//! Quake III team-arena: server commands.
//!
//! Donor provenance: `src/content/q3/team-arena/server-commands.ts`.

use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::format::{game_format, GameFormatArgument};
use crate::q3::base::game::numeric::game_atoi;
use crate::q3::base::game::state::ConnectionState;
use crate::q3::base::shared::definitions::*;
use crate::q3::team_arena::commands::*;
use crate::q3::team_arena::support::*;

// ---------------------------------------------------------------------------
// server-commands.ts
// ---------------------------------------------------------------------------

/// Optional server capability (`ServerCommandCapability`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub enum ServerCommandCapability {
    /// Capability available.
    Available {
        /// Run the command.
        run: Rc<dyn Fn(&[String])>,
    },
    /// Capability unavailable.
    Unavailable {
        /// Reason.
        reason: String,
    },
}

/// Server-command cvar name (`ServerCommandCvar`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerCommandCvar {
    /// Ban list.
    GBanIPs,
    /// Ban/allow mode.
    GFilterBan,
    /// Game type.
    GGametype,
    /// Dedicated.
    Dedicated,
}

impl ServerCommandCvar {
    /// Cvar spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ServerCommandCvar::GBanIPs => "g_banIPs",
            ServerCommandCvar::GFilterBan => "g_filterBan",
            ServerCommandCvar::GGametype => "g_gametype",
            ServerCommandCvar::Dedicated => "dedicated",
        }
    }
}

/// Server-command services (`GameServerCommandHost`).
pub trait GameServerCommandHost {
    /// Read a VM cvar.
    fn read_vm_cvar(&self, name: ServerCommandCvar) -> CvarSnapshot;
    /// Print a line.
    fn print(&self, text: &str);
    /// Send a server command.
    fn send_server_command(&self, client_num: i32, text: &str);
    /// Execute a console command immediately.
    fn execute_console_now(&self, text: &str);
    /// Set a team.
    fn set_team(&self, entity: &EntityRef, team: &str);
    /// Bot services.
    fn bots(&self) -> ServerCommandCapability;
    /// Memory services.
    fn memory(&self) -> ServerCommandCapability;
    /// Podium services.
    fn podium(&self) -> ServerCommandCapability;
}

pub(crate) const MAX_IP_FILTERS: usize = 1024;

pub(crate) const MAX_BAN_STRING: usize = 256;

pub(crate) const FREE_FILTER: u32 = 0xffff_ffff;

/// IP filter (`IpFilter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpFilter {
    /// Mask.
    pub mask: u32,
    /// Comparison.
    pub compare: u32,
}

/// Server-command state (`GameServerCommandState`).
#[derive(Debug, Default)]
pub struct GameServerCommandState {
    /// Active filters.
    pub filters: RefCell<Vec<IpFilter>>,
}

/// Cached ban cvar string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BanVmString {
    /// Modification count.
    pub modification_count: i32,
    /// Value.
    pub value: String,
}

pub(crate) fn byte_string(text: &str) -> String {
    let end = text.find('\0').unwrap_or(text.len());
    let value = &text[..end];
    for ch in value.chars() {
        if ch as u32 > 255 {
            panic!("Game console strings require source bytes");
        }
    }
    value.to_string()
}

pub(crate) fn is_digit(character: char) -> bool {
    character.is_ascii_digit()
}

/// `trap_Argv` argument read (`argument`).
pub(crate) fn server_argument(argv: &[String], index: usize) -> String {
    match argv.get(index) {
        Some(value) => byte_string(value).chars().take(1023).collect(),
        None => String::new(),
    }
}

pub(crate) fn string_to_filter(text: &str, print: &dyn Fn(&str)) -> Option<IpFilter> {
    let chars: Vec<char> = text.chars().collect();
    let mut offset = 0usize;
    let mut mask = 0i32;
    let mut compare = 0i32;
    for i in 0..4 {
        let first = chars.get(offset).copied().unwrap_or('\0');
        if !is_digit(first) {
            if first == '*' {
                offset += 1;
                if offset == chars.len() {
                    break;
                }
                offset += 1;
                continue;
            }
            print(&format!(
                "Bad filter address: {}\n",
                chars[offset..].iter().collect::<String>()
            ));
            return None;
        }
        let start = offset;
        while chars.get(offset).copied().unwrap_or('\0') != '\0' && is_digit(chars[offset]) {
            offset += 1;
        }
        if offset - start >= 128 {
            panic!("IP filter component exceeds the source127-digit scratch buffer");
        }
        let digits: String = chars[start..offset].iter().collect();
        compare |= (game_atoi(&digits).unwrap() & 255) << (i * 8);
        mask |= 255 << (i * 8);
        if offset == chars.len() {
            break;
        }
        offset += 1;
    }
    Some(IpFilter {
        mask: mask as u32,
        compare: compare as u32,
    })
}

/// Server-command runtime (`GameServerCommandRuntime`).
pub struct GameServerCommandRuntime {
    /// Entity pool.
    pub pool: PoolRef,
    /// Cvar registry.
    pub cvars: Rc<dyn CvarRegistry>,
    /// Host services.
    pub host: Rc<dyn GameServerCommandHost>,
    state: GameServerCommandState,
    ban_vm_string: RefCell<Option<BanVmString>>,
}

impl GameServerCommandRuntime {
    /// Fresh runtime.
    pub fn new(
        pool: PoolRef,
        cvars: Rc<dyn CvarRegistry>,
        host: Rc<dyn GameServerCommandHost>,
        state: GameServerCommandState,
    ) -> Self {
        Self {
            pool,
            cvars,
            host,
            state,
            ban_vm_string: RefCell::new(None),
        }
    }

    /// Checkpoint capture.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveValue {
        SaveValue::map(vec![
            (
                "filters",
                SaveValue::List(
                    self.state
                        .filters
                        .borrow()
                        .iter()
                        .map(|filter| {
                            SaveValue::map(vec![
                                ("mask", SaveValue::Int(filter.mask as i64)),
                                ("compare", SaveValue::Int(filter.compare as i64)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "banVmString",
                match &*self.ban_vm_string.borrow() {
                    Some(ban) => SaveValue::map(vec![
                        ("modificationCount", SaveValue::Int(ban.modification_count as i64)),
                        ("value", SaveValue::Str(ban.value.clone())),
                    ]),
                    None => SaveValue::Null,
                },
            ),
        ])
    }

    /// Checkpoint restore.
    pub fn restore_save_state(&self, value: &SaveValue) {
        let reader = SaveReader::new(value, "q3.serverCommands");
        let filters: Vec<IpFilter> = reader.field("filters").list(|entry| IpFilter {
            mask: entry.field("mask").integer(0) as u32,
            compare: entry.field("compare").integer(0) as u32,
        });
        let ban = reader.field("banVmString").nullable(|entry| BanVmString {
            modification_count: entry.field("modificationCount").integer(i64::MIN) as i32,
            value: entry.field("value").string(),
        });
        if filters.len() > MAX_IP_FILTERS
            || filters
                .iter()
                .any(|filter| filter.mask as u64 > 0xffff_ffff || filter.compare as u64 > 0xffff_ffff)
        {
            reader.fail("too many IP filters");
        }
        *self.state.filters.borrow_mut() = filters;
        *self.ban_vm_string.borrow_mut() = ban;
    }

    /// Write the ban list cvar (`updateIPBans`).
    pub fn update_ip_bans(&self) {
        let mut list = String::new();
        for filter in self.state.filters.borrow().iter() {
            if filter.compare == FREE_FILTER {
                continue;
            }
            let mut address = String::new();
            for byte in 0..4 {
                if ((filter.mask >> (byte * 8)) & 255) == 255 {
                    address += &((filter.compare >> (byte * 8)) & 255).to_string();
                } else {
                    address += "*";
                }
                address += if byte < 3 { "." } else { " " };
            }
            if list.len() + address.len() >= MAX_BAN_STRING {
                self.host.print("g_banIPs overflowed at MAX_CVAR_VALUE_STRING\n");
                break;
            }
            list += &address;
        }
        self.cvars.set("g_banIPs", &list, true);
    }

    fn add_ip(&self, text: &str) {
        let mut index = self
            .state
            .filters
            .borrow()
            .iter()
            .position(|filter| filter.compare == FREE_FILTER)
            .map(|index| index as i32)
            .unwrap_or(-1);
        if index < 0 {
            index = self.state.filters.borrow().len() as i32;
        }
        if index as usize == MAX_IP_FILTERS {
            self.host.print("IP filter list is full\n");
            return;
        }
        // Undefined native scratch overflow rejects before changing the selected slot.
        let print = |text: &str| self.host.print(text);
        let parsed = string_to_filter(text, &print);
        if index as usize == self.state.filters.borrow().len() {
            self.state.filters.borrow_mut().push(IpFilter {
                mask: 0,
                compare: FREE_FILTER,
            });
        }
        let mut filters = self.state.filters.borrow_mut();
        let selected = match filters.get_mut(index as usize) {
            Some(selected) => selected,
            None => panic!("Missing selected IP filter slot"),
        };
        match parsed {
            None => selected.compare = FREE_FILTER,
            Some(parsed) => {
                selected.mask = parsed.mask;
                selected.compare = parsed.compare;
            }
        }
        drop(filters);
        self.update_ip_bans();
    }

    /// Reload bans from the cvar (`processIPBans`).
    pub fn process_ip_bans(&self) {
        let snapshot = self.host.read_vm_cvar(ServerCommandCvar::GBanIPs);
        let stale = match &*self.ban_vm_string.borrow() {
            Some(ban) => ban.modification_count != snapshot.modification_count,
            None => true,
        };
        if stale {
            *self.ban_vm_string.borrow_mut() = Some(BanVmString {
                modification_count: snapshot.modification_count,
                value: byte_string(&snapshot.value),
            });
        }
        let input = self
            .ban_vm_string
            .borrow()
            .clone()
            .map(|ban| ban.value)
            .unwrap_or_default();
        if input.len() >= MAX_BAN_STRING {
            panic!("g_banIPs exceeds the source256-byte vmCvar buffer");
        }
        // The source writes NULs into its vmCvar string, not its unused local copy.
        if let Some(first_space) = input.find(' ') {
            if let Some(ban) = self.ban_vm_string.borrow_mut().as_mut() {
                ban.value = input[..first_space].to_string();
            }
        }
        let mut start = 0;
        while start < input.len() {
            let space = input[start..].find(' ').map(|space| start + space);
            let Some(space) = space else {
                break;
            };
            if space > start {
                self.add_ip(&input[start..space]);
            }
            start = space;
            while input[start..].starts_with(' ') {
                start += 1;
            }
        }
    }

    /// True rejects admission (`filterPacket`).
    pub fn filter_packet(&self, from: &str) -> bool {
        let text = byte_string(from);
        if text.len() > 1023 {
            panic!("IP address exceeds the source userinfo bound");
        }
        let chars: Vec<char> = text.chars().collect();
        let mut offset = 0usize;
        let mut initialized = 0u32;
        let mut incoming = 0i32;
        while offset < chars.len() && initialized < 4 {
            let mut byte = 0i32;
            while offset < chars.len() && is_digit(chars[offset]) {
                byte = (byte
                    .wrapping_mul(10)
                    .wrapping_add(chars[offset] as i32)
                    .wrapping_sub(48))
                    & 255;
                offset += 1;
            }
            incoming |= byte << (initialized * 8);
            initialized += 1;
            if offset == chars.len() || chars[offset] == ':' {
                break;
            }
            offset += 1;
        }
        let deny_matches = self.host.read_vm_cvar(ServerCommandCvar::GFilterBan).integer_value != 0;
        let initialized_mask = if initialized == 4 {
            0xffff_ffffu32
        } else {
            (2u64.pow(initialized * 8) - 1) as u32
        };
        for filter in self.state.filters.borrow().iter() {
            if (incoming as u32 & filter.mask & initialized_mask) != (filter.compare & initialized_mask) {
                continue;
            }
            // Bots have no IP epair. Missing octets matter only to a matching filter
            // that actually consumes them, never to the empty filter list.
            if filter.mask & !initialized_mask != 0 {
                panic!("IP filter reads uninitialized source octets");
            }
            if (incoming as u32 & filter.mask) == filter.compare {
                return deny_matches;
            }
        }
        !deny_matches
    }

    fn remove_ip(&self, argv: &[String]) {
        if argv.len() < 2 {
            self.host.print("Usage:  sv removeip <ip-mask>\n");
            return;
        }
        let text = server_argument(argv, 1);
        let print = |text: &str| self.host.print(text);
        let parsed = match string_to_filter(&text, &print) {
            Some(parsed) => parsed,
            None => return,
        };
        let found = self
            .state
            .filters
            .borrow()
            .iter()
            .position(|filter| filter.mask == parsed.mask && filter.compare == parsed.compare);
        if let Some(index) = found {
            self.state.filters.borrow_mut()[index].compare = FREE_FILTER;
            self.host.print("Removed.\n");
            self.update_ip_bans();
            return;
        }
        self.host.print(&format!("Didn't find {text}.\n"));
    }

    /// Resolve a console client reference (`clientForString`).
    pub fn client_for_string(&self, value: &str) -> Option<ClientRef> {
        let text = byte_string(value);
        if is_digit(text.chars().next().unwrap_or('\0')) {
            let slot = game_atoi(&text).unwrap();
            if slot < 0 || slot as usize >= self.pool.max_clients() {
                self.host
                    .print(&game_format("Bad client slot: %i\n", &[GameFormatArgument::Int(slot)]));
                return None;
            }
            let client = self.pool.client_at(slot as usize);
            if client.borrow().pers.connected == ConnectionState::Disconnected as i32 {
                self.host.print(&game_format(
                    "Client %i is not connected\n",
                    &[GameFormatArgument::Int(slot)],
                ));
                return None;
            }
            return Some(client);
        }
        for index in 0..self.pool.max_clients() {
            let client = self.pool.client_at(index);
            let record = client.borrow();
            if record.pers.connected != ConnectionState::Disconnected as i32
                && cmd_lower(&record.pers.netname) == cmd_lower(&text)
            {
                return Some(client.clone());
            }
        }
        self.host.print(&format!("User {text} is not on the server\n"));
        None
    }

    fn entity_list(&self) {
        let labels: [(i32, &str); 12] = [
            (EntityType::EtGeneral as i32, "ET_GENERAL"),
            (EntityType::EtPlayer as i32, "ET_PLAYER"),
            (EntityType::EtItem as i32, "ET_ITEM"),
            (EntityType::EtMissile as i32, "ET_MISSILE"),
            (EntityType::EtMover as i32, "ET_MOVER"),
            (EntityType::EtBeam as i32, "ET_BEAM"),
            (EntityType::EtPortal as i32, "ET_PORTAL"),
            (EntityType::EtSpeaker as i32, "ET_SPEAKER"),
            (EntityType::EtPushTrigger as i32, "ET_PUSH_TRIGGER"),
            (EntityType::EtTeleportTrigger as i32, "ET_TELEPORT_TRIGGER"),
            (EntityType::EtInvisible as i32, "ET_INVISIBLE"),
            (EntityType::EtGrapple as i32, "ET_GRAPPLE"),
        ];
        for index in 1..self.pool.num_entities() {
            let entity = self.pool.at(index);
            if !entity.borrow().inuse {
                continue;
            }
            self.host
                .print(&game_format("%3i:", &[GameFormatArgument::Int(index as i32)]));
            let e_type = entity.borrow().s.e_type;
            match labels.iter().find(|label| label.0 == e_type) {
                Some(label) => self.host.print(&format!("{:<20}", label.1)),
                None => self
                    .host
                    .print(&game_format("%3i                 ", &[GameFormatArgument::Int(e_type)])),
            }
            if let Some(classname) = entity.borrow().classname() {
                self.host.print(&classname);
            }
            self.host.print("\n");
        }
    }

    fn capability(&self, service: &ServerCommandCapability, argv: &[String]) {
        match service {
            ServerCommandCapability::Unavailable { reason } => {
                panic!("{} unavailable: {reason}", server_argument(argv, 0))
            }
            ServerCommandCapability::Available { run } => run(argv),
        }
    }

    /// Dispatch a console command (`consoleCommand`).
    pub fn console_command(&self, argv: &[String]) -> bool {
        match cmd_lower(&server_argument(argv, 0)).as_str() {
            "entitylist" => {
                self.entity_list();
                return true;
            }
            "forceteam" => {
                let client = self.client_for_string(&server_argument(argv, 1));
                if let Some(client) = client {
                    let slot = match self.pool.client_index(&client) {
                        Some(slot) => slot,
                        None => panic!("Console client belongs to another entity pool"),
                    };
                    self.host.set_team(&self.pool.at(slot), &server_argument(argv, 2));
                }
                return true;
            }
            "game_memory" => {
                let service = self.host.memory();
                self.capability(&service, argv);
                return true;
            }
            "addbot" | "botlist" => {
                let service = self.host.bots();
                self.capability(&service, argv);
                return true;
            }
            "abort_podium" => {
                if self.host.read_vm_cvar(ServerCommandCvar::GGametype).integer_value == 2 {
                    let service = self.host.podium();
                    self.capability(&service, argv);
                }
                return true;
            }
            "addip" => {
                if argv.len() < 2 {
                    self.host.print("Usage:  addip <ip-mask>\n");
                } else {
                    self.add_ip(&server_argument(argv, 1));
                }
                return true;
            }
            "removeip" => {
                self.remove_ip(argv);
                return true;
            }
            "listip" => {
                self.host.execute_console_now("g_banIPs\n");
                return true;
            }
            _ => {}
        }
        if self.host.read_vm_cvar(ServerCommandCvar::Dedicated).integer_value == 0 {
            return false;
        }
        let start = if cmd_lower(&server_argument(argv, 0)) == "say" {
            1
        } else {
            0
        };
        self.host.send_server_command(
            -1,
            &game_format(
                "print \"server: %s\"",
                &[GameFormatArgument::Text(concat_command_args(argv, start))],
            ),
        );
        true
    }
}
