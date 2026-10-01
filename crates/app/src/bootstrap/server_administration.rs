//! Dedicated-server operator state: filters, masters, rcon limits.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/server-administration.ts`
//! (`ServerOperatorHost`, `ServerOperatorState`, `SourceAdministrationOptions`,
//! `sourceAdministrationCommandNames`, `registerSourceAdministrationCvars`,
//! `sourceServerAdministration`). Session state survives world replacement;
//! native Q3 retains its own game filter list. Filter lists, addresses, and
//! the administration trait reuse `qa_net`; cvars reuse `qa_core`; the
//! filter profile reuses [`ConfigStore`]. Sync port: DNS, config writes, and
//! Q3 master refresh run inline with the donor's texts and generation guards.

use std::collections::HashMap;

use qa_core::cmd::Dialect;
use qa_core::cvar::flags;
use qa_core::cvar::q2_flags;
use qa_core::cvar::CvarRegistry;
use qa_core::numeric::native_atoi;
use qa_net::common::endpoint::address_key;
use qa_net::common::endpoint::ipv4_address;
use qa_net::common::endpoint::resolve_address;
use qa_net::common::endpoint::NetworkAddress;
use qa_net::common::endpoint::ResolveFamily;
use qa_net::q2_net::q2_out_of_band;
use qa_net::services::admin::source_ipv4_filter;
use qa_net::services::admin::IpFilterList;
use qa_net::services::admin::Ipv4Filter;
use qa_net::services::admin::RconResult;
use qa_net::services::admin::ServerAdministration;
use thiserror::Error;

use crate::settings::config::ConfigStore;
use crate::settings::json::parse_json;
use crate::settings::json::stringify;
use crate::settings::json::Json;

/// Server administration failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServerAdministrationError {
    /// Filter profile is invalid.
    #[error("Invalid server filter profile")]
    BadProfile,
    /// Saved filter entry is invalid.
    #[error("Invalid saved server filter")]
    BadFilter,
    /// Rcon before administration cvars.
    #[error("Register source administration cvars before accepting rcon")]
    RconBeforeCvars,
    /// setmaster on a cvar-master source.
    #[error("This source uses master cvars instead of setmaster")]
    CvarMasters,
    /// Deferred Q3 master failure.
    #[error("{0}")]
    MasterFailure(String),
    /// Address failure.
    #[error("{0}")]
    Address(String),
    /// Settings store failure.
    #[error("{0}")]
    Store(String),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] qa_core::cvar::CvarError),
}

/// Maximum saved filters.
const MAX_FILTERS: usize = 1024;

fn filter_text(filter: &Ipv4Filter) -> String {
    [0, 8, 16, 24]
        .iter()
        .map(|shift| ((filter.compare >> shift) & 255).to_string())
        .collect::<Vec<_>>()
        .join(".")
}

/// Parse an unsigned rate field (donor `rateUnsigned`).
fn rate_unsigned(text: &str, offset: usize) -> (u32, usize) {
    let bytes = text.as_bytes();
    let mut cursor = offset.min(bytes.len());
    while cursor < bytes.len() && matches!(bytes[cursor], b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ') {
        cursor += 1;
    }
    let negative = bytes.get(cursor) == Some(&b'-');
    if matches!(bytes.get(cursor), Some(b'+') | Some(b'-')) {
        cursor += 1;
    }
    let start = cursor;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        cursor += 1;
    }
    if start == cursor {
        return (0, offset);
    }
    let magnitude: u64 = text[start..cursor].parse().unwrap_or(u64::MAX);
    let value = if negative {
        0u32.wrapping_sub(magnitude as u32)
    } else {
        magnitude as u32
    };
    (value, cursor)
}

/// Rate credits (donor `rateCredits`).
fn rate_credits(rate: u64) -> u32 {
    let credits = if rate > 0xffff_ffff / 32000 {
        rate / 10000 * 32000
    } else {
        rate * 32000 / 10000
    };
    credits as u32
}

/// Operator host (donor `ServerOperatorHost`).
pub trait ServerOperatorHost {
    /// Command dialect.
    fn dialect(&self) -> Dialect;
    /// Source cvars.
    fn cvars(&self) -> &CvarRegistry;
    /// Whether the server is dedicated.
    fn dedicated(&self) -> bool;
    /// Print diagnostics.
    fn print(&mut self, text: &str);
    /// Send a datagram.
    fn send(&mut self, to: &NetworkAddress, bytes: &[u8]) -> bool;
    /// Write a config file.
    fn write_config(&mut self, name: &str, text: &str);
    /// Send heartbeats.
    fn heartbeat(&mut self);
    /// Resolve a master name (donor `resolveAddress`, sync here).
    fn resolve(&mut self, name: &str, port: u32) -> Result<NetworkAddress, String> {
        resolve_address(name, port, ResolveFamily::V4).map_err(|error| error.to_string())
    }
}

/// Session operator state.
pub struct ServerOperatorState {
    filters: IpFilterList,
    master_addresses: HashMap<String, Vec<NetworkAddress>>,
    master_names: String,
    generation: u64,
    closed: bool,
    master_failure: Option<String>,
    limited_prefixes: Vec<String>,
    rcon_rate_owner: Option<usize>,
    rcon_rate_revision: u32,
    rcon_rate_text: Option<String>,
    rcon_rate_time: u32,
    rcon_credit: u32,
    rcon_credit_cap: u32,
    rcon_cost: u32,
    store: ConfigStore,
    scope: String,
}

impl ServerOperatorState {
    /// Open state, loading the saved filter profile.
    pub fn open(store: ConfigStore, scope: &str) -> Result<Self, ServerAdministrationError> {
        let mut state = Self {
            filters: IpFilterList::default(),
            master_addresses: HashMap::new(),
            master_names: String::new(),
            generation: 0,
            closed: false,
            master_failure: None,
            limited_prefixes: Vec::new(),
            rcon_rate_owner: None,
            rcon_rate_revision: 0,
            rcon_rate_text: None,
            rcon_rate_time: 0,
            rcon_credit: 0,
            rcon_credit_cap: 0,
            rcon_cost: 0,
            store,
            scope: scope.to_string(),
        };
        let Some(text) = state
            .store
            .load_text(scope)
            .map_err(|error| ServerAdministrationError::Store(error.to_string()))?
        else {
            return Ok(state);
        };
        let value = parse_json(&text).map_err(|_| ServerAdministrationError::BadProfile)?;
        let Json::Object(_) = value else {
            return Err(ServerAdministrationError::BadProfile);
        };
        let version_ok = matches!(value.get("version"), Some(Json::Number(version)) if *version == 1.0);
        let Some(Json::Array(filters)) = value.get("filters") else {
            return Err(ServerAdministrationError::BadProfile);
        };
        if !version_ok || filters.len() > MAX_FILTERS {
            return Err(ServerAdministrationError::BadProfile);
        }
        for entry in filters {
            let Json::String(text) = entry else {
                return Err(ServerAdministrationError::BadFilter);
            };
            let Some(filter) = source_ipv4_filter(text) else {
                return Err(ServerAdministrationError::BadFilter);
            };
            state.filters.add(filter);
        }
        Ok(state)
    }

    fn save_filters(&self) -> Result<(), ServerAdministrationError> {
        let mut text = stringify(&Json::Object(vec![
            ("version".to_string(), Json::Number(1.0)),
            (
                "filters".to_string(),
                Json::Array(
                    self.filters
                        .snapshot()
                        .iter()
                        .map(|filter| Json::String(filter_text(filter)))
                        .collect(),
                ),
            ),
        ]));
        text.push('\n');
        self.store
            .dump(&self.scope, &text)
            .map_err(|error| ServerAdministrationError::Store(error.to_string()))
    }

    /// Limited rcon password and prefixes.
    #[must_use]
    pub fn limited_rcon(&self, cvars: &CvarRegistry) -> (String, Vec<String>) {
        (cvars.variable_string("lrcon_password"), self.limited_prefixes.clone())
    }

    /// Handle an `lrconcmd` command; false means unhandled.
    pub fn limited_rcon_command(&mut self, name: &str, raw_args: &str, print: &mut dyn FnMut(&str)) -> bool {
        if name != "addlrconcmd" && name != "dellrconcmd" && name != "listlrconcmds" {
            return false;
        }
        if name == "listlrconcmds" {
            if self.limited_prefixes.is_empty() {
                print("No lrconcmds registered.\n");
            } else {
                let mut text = "id command\n-- -------\n".to_string();
                for (index, prefix) in self.limited_prefixes.iter().enumerate() {
                    text.push_str(&format!("{:2} {prefix}\n", index + 1));
                }
                print(&text);
            }
            return true;
        }
        if raw_args.is_empty() {
            print(&format!(
                "Usage: {name} {}\n",
                if name == "addlrconcmd" {
                    "<command>"
                } else {
                    "<id|cmd|all>"
                }
            ));
            return true;
        }
        let value = if raw_args.starts_with('"') && raw_args.ends_with('"') && raw_args.len() >= 2 {
            &raw_args[1..raw_args.len() - 1]
        } else {
            raw_args
        };
        if name == "addlrconcmd" {
            if self.limited_prefixes.iter().any(|prefix| prefix == value) {
                print(&format!("Lrconcmd already exists: {value}\n"));
            } else {
                self.limited_prefixes.push(value.to_string());
            }
            return true;
        }
        if self.limited_prefixes.is_empty() {
            print("No lrconcmds registered.\n");
            return true;
        }
        if value == "all" {
            self.limited_prefixes.clear();
            return true;
        }
        let numbered = !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
        let index = if numbered {
            native_atoi(value).unwrap_or(0) - 1
        } else {
            self.limited_prefixes
                .iter()
                .position(|prefix| prefix == value)
                .map_or(-1, |index| index as i32)
        };
        if index < 0 || index as usize >= self.limited_prefixes.len() {
            if numbered {
                print(&format!(
                    "No such lrconcmd index: {}\n",
                    native_atoi(value).unwrap_or(0)
                ));
            } else {
                print(&format!("No such lrconcmd string: {value}\n"));
            }
        } else {
            self.limited_prefixes.remove(index as usize);
        }
        true
    }

    fn initialize_rcon_rate(&mut self, text: &str, now_ms: u64, print: &mut dyn FnMut(&str)) {
        let bytes = text.as_bytes();
        let (limit, mut cursor) = rate_unsigned(text, 0);
        let mut period: u32 = 1;
        let mut multiplier: u64 = 1;
        if bytes.get(cursor) == Some(&b'/') {
            let (parsed, end) = rate_unsigned(text, cursor + 1);
            period = if parsed == 0 { 1 } else { parsed };
            cursor = end;
            match bytes.get(cursor).map(|byte| byte.to_ascii_lowercase()) {
                Some(b's') => {
                    multiplier = 1;
                    cursor += 1;
                }
                Some(b'm') => {
                    multiplier = 60;
                    cursor += 1;
                }
                Some(b'h') => {
                    multiplier = 3600;
                    cursor += 1;
                }
                _ => {}
            }
        }
        if limit == 0 {
            self.rcon_rate_time = 0;
            self.rcon_credit = 0;
            self.rcon_credit_cap = 0;
            self.rcon_cost = 0;
            return;
        }
        if u64::from(period) > 0xffff_ffff / (10000 * multiplier) {
            print(&format!("Period too large: {period}\n"));
            return;
        }
        let rate = 10000 * u64::from(period) * multiplier / u64::from(limit);
        if rate == 0 {
            print(&format!("Limit too large: {limit}\n"));
            return;
        }
        let burst = match text[cursor.min(text.len())..].find('*') {
            None => 5,
            Some(star) => rate_unsigned(text, cursor + star + 1).0,
        };
        if u64::from(burst) > 0xffff_ffff / rate {
            print(&format!("Burst too large: {burst}\n"));
            return;
        }
        self.rcon_rate_time = now_ms as u32;
        self.rcon_credit = rate_credits(rate * u64::from(burst));
        self.rcon_credit_cap = self.rcon_credit;
        self.rcon_cost = rate_credits(rate);
    }

    /// Token-bucket rcon gate.
    pub fn rcon_rate_allowed(
        &mut self,
        cvars: &CvarRegistry,
        now_ms: u64,
        print: &mut dyn FnMut(&str),
    ) -> Result<bool, ServerAdministrationError> {
        let Some(setting) = cvars.get("sv_rcon_limit") else {
            return Err(ServerAdministrationError::RconBeforeCvars);
        };
        let owner = cvars as *const CvarRegistry as usize;
        if self.rcon_rate_text.as_deref() != Some(setting.value.as_str())
            || (self.rcon_rate_owner == Some(owner) && self.rcon_rate_revision != setting.modification_count)
        {
            self.initialize_rcon_rate(&setting.value, now_ms, print);
        }
        self.rcon_rate_owner = Some(owner);
        self.rcon_rate_revision = setting.modification_count;
        self.rcon_rate_text = Some(setting.value.clone());
        let time = now_ms as u32;
        self.rcon_credit = self
            .rcon_credit
            .wrapping_add(time.wrapping_sub(self.rcon_rate_time).wrapping_mul(32));
        self.rcon_rate_time = time;
        if self.rcon_credit > self.rcon_credit_cap {
            self.rcon_credit = self.rcon_credit_cap;
        }
        if self.rcon_credit < self.rcon_cost {
            return Ok(false);
        }
        self.rcon_credit -= self.rcon_cost;
        Ok(true)
    }

    /// Refund one rcon cost.
    pub fn recharge_rcon_rate(&mut self) {
        self.rcon_credit = (self.rcon_credit.wrapping_add(self.rcon_cost)).min(self.rcon_credit_cap);
    }

    /// Master servers for a dialect.
    #[must_use]
    pub fn masters(&self, dialect: Dialect) -> Vec<NetworkAddress> {
        match dialect {
            Dialect::Q1Netquake => Vec::new(),
            Dialect::Q2Classic | Dialect::Q2Rerelease => {
                let mut masters = vec![ipv4_address([192, 246, 40, 37], 27900, false).expect("valid master")];
                masters.extend(self.master_addresses.get("q2").cloned().unwrap_or_default());
                masters
            }
            Dialect::Q3 => self.master_addresses.get("q3").cloned().unwrap_or_default(),
            Dialect::Q1Quakeworld => self.master_addresses.get("qw").cloned().unwrap_or_default(),
        }
    }

    /// Whether an address is rejected under `filterban`.
    #[must_use]
    pub fn rejects(&self, address: &NetworkAddress, filterban: f32) -> bool {
        let NetworkAddress::Ipv4 { host, .. } = address else {
            return false;
        };
        let incoming = u32::from_le_bytes(*host);
        let matches = self
            .filters
            .snapshot()
            .iter()
            .any(|filter| incoming & filter.mask == filter.compare);
        if matches {
            filterban.trunc() != 0.0
        } else {
            filterban == 0.0
        }
    }

    /// Handle a filter command; false means unhandled.
    pub fn filter_command<Host: ServerOperatorHost>(
        &mut self,
        host: &mut Host,
        name: &str,
        args: &[String],
    ) -> Result<bool, ServerAdministrationError> {
        if host.dialect() == Dialect::Q3 {
            return Ok(false);
        }
        let (name, args) = if name == "sv" && host.dialect().is_q2() {
            (
                args.first().map_or("", String::as_str),
                args.get(1..).unwrap_or_default(),
            )
        } else {
            (name, args)
        };
        if name != "addip" && name != "removeip" && name != "listip" && name != "writeip" {
            return Ok(false);
        }
        if name == "listip" {
            let mut text = "Filter list:\n".to_string();
            text.push_str(
                &self
                    .filters
                    .snapshot()
                    .iter()
                    .map(filter_text)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            text.push('\n');
            host.print(&text);
            return Ok(true);
        }
        if name == "writeip" {
            let prefix = if host.dialect().is_q2() { "sv " } else { "" };
            let mut text = format!(
                "set filterban {}\n",
                host.cvars().variable_value("filterban").trunc() as i64
            );
            for filter in self.filters.snapshot() {
                text.push_str(&format!("{prefix}addip {}\n", filter_text(&filter)));
            }
            host.write_config("listip.cfg", &text);
            host.print("Wrote listip.cfg\n");
            return Ok(true);
        }
        let Some(text) = args.first() else {
            host.print(&format!("Usage: {name} <ip-mask>\n"));
            return Ok(true);
        };
        let Some(filter) = source_ipv4_filter(text) else {
            host.print(&format!("Bad filter address: {text}\n"));
            return Ok(true);
        };
        if name == "removeip" {
            let removed = self.filters.remove(&filter);
            if removed {
                self.save_filters()?;
            }
            host.print(if removed { "Removed.\n" } else { "Didn't find.\n" });
            return Ok(true);
        }
        if self.filters.snapshot().len() >= MAX_FILTERS {
            host.print("IP filter list is full\n");
            return Ok(true);
        }
        self.filters.add(filter);
        self.save_filters()?;
        Ok(true)
    }

    /// Resolve and ping master servers.
    pub fn set_masters<Host: ServerOperatorHost>(
        &mut self,
        host: &mut Host,
        names: &[String],
    ) -> Result<(), ServerAdministrationError> {
        if self.closed {
            return Ok(());
        }
        let quakeworld = host.dialect() == Dialect::Q1Quakeworld;
        if !quakeworld && !host.dialect().is_q2() {
            return Err(ServerAdministrationError::CvarMasters);
        }
        if !quakeworld && !host.dedicated() {
            host.print("Only dedicated servers use masters.\n");
            return Ok(());
        }
        self.generation += 1;
        let generation = self.generation;
        let mut addresses = Vec::new();
        for name in names.iter().take(if quakeworld { 8 } else { 7 }) {
            if quakeworld && name == "none" {
                break;
            }
            match host.resolve(name, if quakeworld { 27000 } else { 27900 }) {
                Ok(address) => {
                    if generation != self.generation {
                        return Ok(());
                    }
                    addresses.push(address);
                    host.print(&format!(
                        "Master server at {}\nSending a ping.\n",
                        address_key(addresses.last().expect("just pushed"), true)
                    ));
                    host.send(
                        addresses.last().expect("just pushed"),
                        &(if quakeworld {
                            vec![107, 0]
                        } else {
                            q2_out_of_band("ping", false)
                        }),
                    );
                }
                Err(error) => {
                    if generation != self.generation {
                        return Ok(());
                    }
                    host.print(&format!("Bad master address {name}: {error}\n"));
                }
            }
        }
        if generation != self.generation {
            return Ok(());
        }
        self.master_addresses
            .insert(if quakeworld { "qw".to_string() } else { "q2".to_string() }, addresses);
        host.heartbeat();
        Ok(())
    }

    /// Refresh Q3 masters from `sv_master1..5` (resolved inline here).
    pub fn refresh_q3_masters(
        &mut self,
        cvars: &CvarRegistry,
        print: &mut dyn FnMut(&str),
    ) -> Result<(), ServerAdministrationError> {
        if self.closed {
            return Ok(());
        }
        if let Some(failure) = self.master_failure.take() {
            return Err(ServerAdministrationError::MasterFailure(failure));
        }
        let names: Vec<String> = (1..=5)
            .map(|index| cvars.variable_string(&format!("sv_master{index}")))
            .filter(|name| !name.is_empty())
            .collect();
        let key = names.join("\n");
        if key == self.master_names {
            return Ok(());
        }
        self.master_names = key;
        self.generation += 1;
        let generation = self.generation;
        let mut addresses = Vec::new();
        for name in &names {
            match resolve_address(name, 27950, ResolveFamily::V4) {
                Ok(address) => {
                    if generation != self.generation {
                        return Ok(());
                    }
                    addresses.push(address);
                }
                Err(error) => {
                    if generation != self.generation {
                        return Ok(());
                    }
                    print(&format!("Bad master address {name}: {error}\n"));
                }
            }
        }
        if generation == self.generation {
            self.master_addresses.insert("q3".to_string(), addresses);
        }
        Ok(())
    }

    /// Retire the state.
    pub fn close(&mut self) {
        self.closed = true;
        self.generation += 1;
        self.master_failure = None;
    }
}

/// Administration command names for a dialect.
#[must_use]
pub fn source_administration_command_names(dialect: Dialect) -> &'static [&'static str] {
    match dialect {
        Dialect::Q1Netquake => &[],
        Dialect::Q1Quakeworld => &["setmaster", "heartbeat", "addip", "removeip", "listip", "writeip"],
        Dialect::Q2Classic | Dialect::Q2Rerelease => &[
            "setmaster",
            "heartbeat",
            "sv",
            "addlrconcmd",
            "dellrconcmd",
            "listlrconcmds",
        ],
        Dialect::Q3 => &["heartbeat", "addip", "removeip", "listip"],
    }
}

/// Register source administration cvars.
pub fn register_source_administration_cvars(cvars: &mut CvarRegistry) -> Result<(), ServerAdministrationError> {
    let dialect = cvars.dialect();
    let register =
        |cvars: &mut CvarRegistry, name: &str, value: &str, flags: u32| -> Result<(), ServerAdministrationError> {
            if cvars.get(name).is_none() || cvars.is_console_created(name) {
                cvars.register(name, value, flags)?;
            } else {
                cvars.add_flags(name, flags)?;
            }
            Ok(())
        };
    register(
        cvars,
        if dialect == Dialect::Q3 {
            "rconPassword"
        } else {
            "rcon_password"
        },
        "",
        if dialect.is_q2() {
            q2_flags::PRIVATE
        } else if dialect == Dialect::Q3 {
            flags::TEMPORARY
        } else {
            0
        },
    )?;
    if dialect != Dialect::Q3 {
        register(cvars, "filterban", "1", 0)?;
    }
    if dialect.is_q2() {
        register(cvars, "public", "0", q2_flags::LATCH)?;
        register(cvars, "lrcon_password", "", q2_flags::PRIVATE)?;
        register(cvars, "sv_rcon_limit", "1", 0)?;
    }
    if dialect == Dialect::Q3 {
        for index in 1..=5 {
            register(
                cvars,
                &format!("sv_master{index}"),
                if index == 1 { "master.quake3arena.com" } else { "" },
                if index == 1 { 0 } else { flags::ARCHIVE },
            )?;
        }
    }
    Ok(())
}

/// Source administration owners (donor `SourceAdministrationOptions`).
pub struct SourceAdministrationOptions<'state, 'cvars, Execute, Record> {
    /// Command dialect.
    pub dialect: Dialect,
    /// Operator state.
    pub state: &'state ServerOperatorState,
    /// Source cvars.
    pub cvars: &'cvars mut CvarRegistry,
    /// Whether the server is dedicated.
    pub dedicated: bool,
    /// Command execution.
    pub execute: Execute,
    /// Rcon recording.
    pub record: Record,
}

/// Source server administration.
pub struct SourceServerAdministration<'state, 'cvars, Execute, Record> {
    dialect: Dialect,
    state: &'state ServerOperatorState,
    cvars: &'cvars CvarRegistry,
    dedicated: bool,
    execute: Execute,
    record: Record,
}

/// Build source server administration (registers cvars first).
pub fn source_server_administration<'state, 'cvars, Execute, Record>(
    options: SourceAdministrationOptions<'state, 'cvars, Execute, Record>,
) -> Result<SourceServerAdministration<'state, 'cvars, Execute, Record>, ServerAdministrationError>
where
    Execute: FnMut(&str, &mut dyn FnMut(&str)),
    Record: FnMut(&NetworkAddress, RconResult),
{
    register_source_administration_cvars(options.cvars)?;
    Ok(SourceServerAdministration {
        dialect: options.dialect,
        state: options.state,
        cvars: options.cvars,
        dedicated: options.dedicated,
        execute: options.execute,
        record: options.record,
    })
}

impl<Execute, Record> ServerAdministration for SourceServerAdministration<'_, '_, Execute, Record>
where
    Execute: FnMut(&str, &mut dyn FnMut(&str)),
    Record: FnMut(&NetworkAddress, RconResult),
{
    fn rcon_password(&self) -> String {
        self.cvars.variable_string(if self.dialect == Dialect::Q3 {
            "rconPassword"
        } else {
            "rcon_password"
        })
    }

    fn execute(&mut self, command: &str, output: &mut dyn FnMut(&str)) {
        (self.execute)(command, output);
    }

    fn rejects(&self, address: &NetworkAddress) -> bool {
        if self.dialect == Dialect::Q3 {
            return false;
        }
        self.state.rejects(address, self.cvars.variable_value("filterban"))
    }

    fn masters(&self) -> Vec<NetworkAddress> {
        if !self.dedicated
            || (self.dialect.is_q2() && self.cvars.variable_value("public") == 0.0)
            || (self.dialect == Dialect::Q3 && self.cvars.variable_value("dedicated") != 2.0)
        {
            return Vec::new();
        }
        self.state.masters(self.dialect)
    }

    fn record(&mut self, address: &NetworkAddress, result: RconResult) {
        (self.record)(address, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> ConfigStore {
        let root = std::env::temp_dir().join(format!("qa-server-admin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        ConfigStore::new(root)
    }

    struct FakeHost {
        dialect: Dialect,
        cvars: CvarRegistry,
        dedicated: bool,
        printed: Vec<String>,
        sent: Vec<(NetworkAddress, Vec<u8>)>,
        configs: Vec<(String, String)>,
        heartbeats: usize,
    }

    impl ServerOperatorHost for FakeHost {
        fn dialect(&self) -> Dialect {
            self.dialect
        }

        fn cvars(&self) -> &CvarRegistry {
            &self.cvars
        }

        fn dedicated(&self) -> bool {
            self.dedicated
        }

        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }

        fn send(&mut self, to: &NetworkAddress, bytes: &[u8]) -> bool {
            self.sent.push((
                match to {
                    NetworkAddress::Ipv4 { host, port } => NetworkAddress::Ipv4 {
                        host: *host,
                        port: *port,
                    },
                    other => panic!("unexpected {other:?}"),
                },
                bytes.to_vec(),
            ));
            true
        }

        fn write_config(&mut self, name: &str, text: &str) {
            self.configs.push((name.to_string(), text.to_string()));
        }

        fn heartbeat(&mut self) {
            self.heartbeats += 1;
        }

        fn resolve(&mut self, name: &str, port: u32) -> Result<NetworkAddress, String> {
            if name == "bad.example" {
                return Err("no such host".to_string());
            }
            ipv4_address([203, 0, 113, 7], port, false).map_err(|error| error.to_string())
        }
    }

    fn make_host(dialect: Dialect) -> FakeHost {
        FakeHost {
            dialect,
            cvars: CvarRegistry::new(dialect),
            dedicated: true,
            printed: Vec::new(),
            sent: Vec::new(),
            configs: Vec::new(),
            heartbeats: 0,
        }
    }

    fn prints(host: &FakeHost) -> String {
        host.printed.concat()
    }

    #[test]
    fn rate_parsing_matches_donor() {
        assert_eq!(rate_unsigned("10", 0), (10, 2));
        assert_eq!(rate_unsigned("  -3", 0), (0u32.wrapping_sub(3), 4));
        assert_eq!(rate_unsigned("abc", 0), (0, 0));
        assert_eq!(rate_unsigned("99999999999999999999999", 0), (0xffff_ffff, 23));
        assert_eq!(rate_credits(10000), 32000);
    }

    #[test]
    fn rcon_rate_limits_and_recharges() {
        let mut state = ServerOperatorState::open(store("rcon"), "filters.json").unwrap();
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        let mut sink = |_: &str| {};
        assert_eq!(
            state.rcon_rate_allowed(&cvars, 0, &mut sink),
            Err(ServerAdministrationError::RconBeforeCvars)
        );
        register_source_administration_cvars(&mut cvars).unwrap();
        cvars.set("sv_rcon_limit", "1", true).unwrap();
        assert!(state.rcon_rate_allowed(&cvars, 0, &mut sink).unwrap());
        // Five-burst default at 1/s: five immediate allowances, then throttled.
        for _ in 0..4 {
            assert!(state.rcon_rate_allowed(&cvars, 0, &mut sink).unwrap());
        }
        assert!(!state.rcon_rate_allowed(&cvars, 0, &mut sink).unwrap());
        state.recharge_rcon_rate();
        assert!(state.rcon_rate_allowed(&cvars, 0, &mut sink).unwrap());
        assert!(!state.rcon_rate_allowed(&cvars, 0, &mut sink).unwrap());
    }

    #[test]
    fn limited_rcon_commands_manage_prefixes() {
        let mut state = ServerOperatorState::open(store("lrcon"), "filters.json").unwrap();
        let mut out = Vec::new();
        assert!(!state.limited_rcon_command("say", "", &mut |text| out.push(text.to_string())));
        assert!(state.limited_rcon_command("addlrconcmd", "status", &mut |text| out.push(text.to_string())));
        assert!(state.limited_rcon_command("addlrconcmd", "status", &mut |text| out.push(text.to_string())));
        assert!(state.limited_rcon_command("listlrconcmds", "", &mut |text| out.push(text.to_string())));
        assert!(state.limited_rcon_command("dellrconcmd", "7", &mut |text| out.push(text.to_string())));
        assert!(state.limited_rcon_command("dellrconcmd", "1", &mut |text| out.push(text.to_string())));
        assert!(state.limited_rcon_command("listlrconcmds", "", &mut |text| out.push(text.to_string())));
        let text = out.concat();
        assert!(text.contains("Lrconcmd already exists: status\n"));
        assert!(text.contains("No such lrconcmd index: 7\n"));
        assert!(text.contains("No lrconcmds registered.\n"));
    }

    #[test]
    fn filter_commands_round_trip_and_persist() {
        let store = store("filters");
        let mut state = ServerOperatorState::open(store, "filters.json").unwrap();
        let mut host = make_host(Dialect::Q1Quakeworld);
        assert!(state
            .filter_command(&mut host, "addip", &["192.168.1.0".to_string()])
            .unwrap());
        assert!(state.filter_command(&mut host, "addip", &["nope".to_string()]).unwrap());
        assert!(state.filter_command(&mut host, "listip", &[]).unwrap());
        assert!(state.filter_command(&mut host, "writeip", &[]).unwrap());
        assert!(prints(&host).contains("Filter list:\n192.168.1.0\n"));
        assert!(prints(&host).contains("Bad filter address: nope\n"));
        assert_eq!(host.configs[0].0, "listip.cfg");
        let address = ipv4_address([192, 168, 1, 9], 27500, false).unwrap();
        assert!(state.rejects(&address, 1.0));
        assert!(!state.rejects(&address, 0.0));
        let other = ipv4_address([10, 0, 0, 1], 27500, false).unwrap();
        assert!(!state.rejects(&other, 1.0));
        assert!(state.rejects(&other, 0.0));
        assert!(state
            .filter_command(&mut host, "removeip", &["192.168.1.0".to_string()])
            .unwrap());
        assert!(prints(&host).contains("Removed.\n"));
        assert!(!state
            .filter_command(&mut host, "addip", &["1.2.3.4".to_string()])
            .is_err());
        // Q3 filtering belongs to the game.
        let mut q3 = make_host(Dialect::Q3);
        assert!(!state
            .filter_command(&mut q3, "addip", &["1.2.3.4".to_string()])
            .unwrap());
    }

    #[test]
    fn masters_resolve_and_heartbeat() {
        let mut state = ServerOperatorState::open(store("masters"), "filters.json").unwrap();
        let mut host = make_host(Dialect::Q1Quakeworld);
        state
            .set_masters(&mut host, &["master.example".to_string(), "bad.example".to_string()])
            .unwrap();
        assert_eq!(host.heartbeats, 1);
        assert_eq!(host.sent.len(), 1);
        assert_eq!(host.sent[0].1, vec![107, 0]);
        assert!(prints(&host).contains("Master server at 203.0.113.7:27000\nSending a ping.\n"));
        assert!(prints(&host).contains("Bad master address bad.example: no such host\n"));
        assert_eq!(state.masters(Dialect::Q1Quakeworld).len(), 1);
        assert!(state.masters(Dialect::Q1Netquake).is_empty());
        let mut q3 = make_host(Dialect::Q3);
        assert_eq!(
            state.set_masters(&mut q3, &["x".to_string()]),
            Err(ServerAdministrationError::CvarMasters)
        );
        let mut q2 = make_host(Dialect::Q2Classic);
        q2.dedicated = false;
        state.set_masters(&mut q2, &["x".to_string()]).unwrap();
        assert!(prints(&q2).contains("Only dedicated servers use masters.\n"));
    }

    #[test]
    fn administration_registers_names_and_passwords() {
        assert!(source_administration_command_names(Dialect::Q1Netquake).is_empty());
        assert_eq!(source_administration_command_names(Dialect::Q3).len(), 4);
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        let state = ServerOperatorState::open(store("admin"), "filters.json").unwrap();
        let mut admin = source_server_administration(SourceAdministrationOptions {
            dialect: Dialect::Q2Classic,
            state: &state,
            cvars: &mut cvars,
            dedicated: true,
            execute: |_: &str, _: &mut dyn FnMut(&str)| {},
            record: |_: &NetworkAddress, _: RconResult| {},
        })
        .unwrap();
        assert_eq!(admin.rcon_password(), "");
        assert!(admin.masters().is_empty());
        let mut out = Vec::new();
        admin.execute("status", &mut |text| out.push(text.to_string()));
        let address = ipv4_address([10, 0, 0, 1], 27910, false).unwrap();
        assert!(!admin.rejects(&address));
    }
}
