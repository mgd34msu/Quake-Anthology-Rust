//! Quake III server browser view.
//!
//! Donor provenance: `Q3BrowserSource`, `Q3BrowserCacheRow`,
//! `Q3BrowserCacheList`, `Q3BrowserCacheView`, `Q3BrowserHost`,
//! `Q3BrowserViewOptions`, and `Q3BrowserView` in
//! `src/network/q3/browser-view.ts` (`LAN_*` list and request semantics
//! from `cl_ui.c` / `cl_main.c`). Browser core types reuse
//! [`ServerBrowser`](crate::services::discovery::ServerBrowser),
//! [`ServerStatus`](crate::services::discovery::ServerStatus), and the
//! discovery request types; info strings reuse
//! [`set_info_value`](qa_core::cvar::set_info_value); host async
//! operations become sync blocking calls.
//!
//! The donor host owns the browser core, which cannot be expressed with
//! Rust borrows; here the view borrows the core and the host separately
//! from their owner. Async completion guards (the view generation and
//! the `assertCurrent` callbacks) are TypeScript-only: sync calls cannot
//! complete after close, so they are omitted.

use std::collections::HashSet;

use qa_core::cmd::{source_command_text, Dialect};
use qa_core::cvar::{set_info_value, InfoOptions, InfoTarget};
use qa_core::numeric::native_atoi;
use thiserror::Error;

use crate::common::endpoint::{address_key, NetworkAddress};
use crate::services::discovery::{
    DiscoveryError, DiscoveryRequestHandle, DiscoveryRequestKind, DiscoveryRequestResult, RequestTimeout,
    ServerBrowser, ServerStatus,
};

/// Browser source (`Q3BrowserSource`): 0 local, 1 internet, 2 favorites, 3 history.
pub type Q3BrowserSource = u8;

/// Cached row (`Q3BrowserCacheRow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3BrowserCacheRow {
    /// Server address.
    pub address: NetworkAddress,
    /// Server name.
    pub name: String,
    /// Visibility value.
    pub visible: i32,
    /// Ping.
    pub ping: i32,
}

/// Cached list (`Q3BrowserCacheList`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3BrowserCacheList {
    /// Source (1|2|3).
    pub source: u8,
    /// Rows.
    pub rows: Vec<Q3BrowserCacheRow>,
}

/// Cached view (`Q3BrowserCacheView`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q3BrowserCacheView {
    /// Lists.
    pub lists: Vec<Q3BrowserCacheList>,
}

/// Browser list state (donor `q3List` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3BrowserListState {
    /// Canonical addresses.
    pub addresses: Vec<NetworkAddress>,
    /// Whether the list is still loading.
    pub pending: bool,
    /// List generation.
    pub generation: i32,
    /// Reset generation.
    pub reset_generation: i32,
}

/// Sync browser host (donor `Q3BrowserHost` with blocking calls).
pub trait Q3BrowserHost {
    /// List state for a source.
    fn q3_list(&mut self, source: Q3BrowserSource) -> Q3BrowserListState;
    /// Resolve address text (blocking).
    fn resolve_q3(&mut self, text: &str) -> Option<NetworkAddress>;
    /// Add an address to a source list.
    fn add_q3(&mut self, source: Q3BrowserSource, address: &NetworkAddress);
    /// Remove an address from a source list.
    fn remove_q3(&mut self, source: Q3BrowserSource, address: &NetworkAddress);
    /// Scan for local servers.
    fn scan_q3(&mut self);
    /// Request masters (blocking).
    fn request_q3_master(&mut self, source: u8, remote: &str, protocol: i32, keywords: &[String]);
    /// Load the cache (blocking).
    fn load_q3_cache(&mut self) -> Option<Q3BrowserCacheView>;
    /// Save the cache (blocking).
    fn save_q3_cache(&mut self, view: &Q3BrowserCacheView);
    /// Panic when the host is closed (donor throws; cf.
    /// [`Q3ClientBindings::assert_current`](crate::q3_net::Q3ClientBindings::assert_current)).
    fn assert_open(&mut self);
}

/// Browser view failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3BrowserError {
    /// Browser core failure.
    #[error("{0}")]
    Discovery(#[from] DiscoveryError),
    /// Command-text failure.
    #[error("{0}")]
    Cmd(#[from] qa_core::cmd::CmdError),
    /// Info-string failure.
    #[error("{0}")]
    Cvar(#[from] qa_core::cvar::CvarError),
    /// Integer-parse failure.
    #[error("{0}")]
    Numeric(#[from] qa_core::numeric::NumericError),
    /// Operation after close.
    #[error("{0}")]
    Closed(&'static str),
    /// Fatal copy failure.
    #[error("{0}")]
    Fatal(&'static str),
    /// Browser failure with donor text.
    #[error("{0}")]
    Browser(String),
}

/// Optional ping-address sink (`write` in `getPing`).
pub type Q3PingAddressSink<'a> = Option<&'a mut dyn FnMut(Option<&str>)>;

/// Ping result (`getPing`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3PingResult {
    /// Address string.
    pub address: String,
    /// Ping time.
    pub time: i32,
}

/// Browser row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    address: Option<NetworkAddress>,
    name: String,
    visible: i32,
    ping: i32,
}

impl Row {
    fn empty() -> Self {
        Self {
            address: None,
            name: String::new(),
            visible: 0,
            ping: 0,
        }
    }
}

/// Browser list.
#[derive(Debug, Clone)]
struct List {
    rows: Vec<Row>,
    seen: HashSet<String>,
    overflow: Vec<NetworkAddress>,
    count: i32,
    generation: i32,
    reset_generation: i32,
    pending: bool,
}

impl List {
    fn new(source: u8) -> Self {
        Self {
            rows: vec![Row::empty(); capacity(source)],
            seen: HashSet::new(),
            overflow: Vec::new(),
            count: 0,
            generation: -1,
            reset_generation: -1,
            pending: false,
        }
    }
}

/// Ping slot.
#[derive(Debug, Clone)]
struct PingSlot {
    address: Option<NetworkAddress>,
    handle: Option<DiscoveryRequestHandle>,
    start: f64,
    info: String,
    published: bool,
}

/// Status slot.
#[derive(Debug, Clone)]
struct StatusSlot {
    address: Option<NetworkAddress>,
    handle: Option<DiscoveryRequestHandle>,
    start: f64,
    retrieved: bool,
    print: bool,
}

/// List capacity (`capacity`).
fn capacity(source: u8) -> usize {
    if source == 2 {
        4096
    } else {
        128
    }
}

/// Address equality (`same`).
fn same(left: Option<&NetworkAddress>, right: Option<&NetworkAddress>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => address_key(left, true) == address_key(right, true),
        _ => false,
    }
}

/// Truncate to a C destination size (`copyString`).
fn copy_string(text: &str, size: usize) -> Result<String, Q3BrowserError> {
    if size < 1 {
        return Err(Q3BrowserError::Fatal("Q_strncpyz: destsize < 1"));
    }
    Ok(utf16_slice(text, size - 1))
}

/// Truncate to UTF-16 units (`slice`).
fn utf16_slice(text: &str, length: usize) -> String {
    String::from_utf16_lossy(&text.encode_utf16().take(length).collect::<Vec<_>>())
}

/// Index a slot array (`slotAt`).
fn slot_at<T>(slots: &[T], index: i32) -> Result<&T, Q3BrowserError> {
    if index < 0 {
        return Err(Q3BrowserError::Browser(format!(
            "Server browser source index {index} outside {}",
            slots.len()
        )));
    }
    slots
        .get(index as usize)
        .ok_or_else(|| Q3BrowserError::Browser(format!("Server browser source index {index} outside {}", slots.len())))
}

/// Case-insensitive byte comparison (`compareText`, `Q_stricmp` order).
fn compare_text(left: &str, right: &str) -> i32 {
    let left: Vec<u16> = left.encode_utf16().collect();
    let right: Vec<u16> = right.encode_utf16().collect();
    let mut index = 0;
    loop {
        let mut a = i32::from(left.get(index).copied().unwrap_or(0));
        let mut b = i32::from(right.get(index).copied().unwrap_or(0));
        a = (a << 24) >> 24;
        b = (b << 24) >> 24;
        if a != b {
            if (97..=122).contains(&a) {
                a -= 32;
            }
            if (97..=122).contains(&b) {
                b -= 32;
            }
            if a != b {
                return if a < b { -1 } else { 1 };
            }
        }
        if a == 0 {
            return 0;
        }
        index += 1;
    }
}

/// Pad to UTF-16 units (`padEnd`).
fn pad_end(text: &str, width: usize) -> String {
    let mut out = text.to_owned();
    let length = text.encode_utf16().count();
    if length < width {
        out.push_str(&" ".repeat(width - length));
    }
    out
}

/// Server browser view (`Q3BrowserView`).
pub struct Q3BrowserView<'a, 'c, 'h>
where
    'a: 'c,
{
    core: &'c mut ServerBrowser<'a>,
    host: &'h mut dyn Q3BrowserHost,
    now: Box<dyn FnMut() -> f64 + 'h>,
    max_ping: Box<dyn FnMut() -> f64 + 'h>,
    status_resend_time: Box<dyn FnMut() -> f64 + 'h>,
    print: Box<dyn FnMut(&str) + 'h>,
    lists: [List; 4],
    pings: [PingSlot; 32],
    statuses: [StatusSlot; 16],
    closed: bool,
}

impl<'a, 'c, 'h> Q3BrowserView<'a, 'c, 'h>
where
    'a: 'c,
{
    /// Build a view over a browser core, host, and callbacks.
    pub fn new(
        core: &'c mut ServerBrowser<'a>,
        host: &'h mut dyn Q3BrowserHost,
        now: impl FnMut() -> f64 + 'h,
        max_ping: impl FnMut() -> f64 + 'h,
        status_resend_time: impl FnMut() -> f64 + 'h,
        print: impl FnMut(&str) + 'h,
    ) -> Self {
        Self {
            core,
            host,
            now: Box::new(now),
            max_ping: Box::new(max_ping),
            status_resend_time: Box::new(status_resend_time),
            print: Box::new(print),
            lists: [List::new(0), List::new(1), List::new(2), List::new(3)],
            pings: std::array::from_fn(|_| PingSlot {
                address: None,
                handle: None,
                start: 0.0,
                info: String::new(),
                published: false,
            }),
            statuses: std::array::from_fn(|_| StatusSlot {
                address: None,
                handle: None,
                start: 0.0,
                retrieved: true,
                print: false,
            }),
            closed: false,
        }
    }

    /// Assert the view is open (`entry`).
    fn entry(&mut self) -> Result<(), Q3BrowserError> {
        if self.closed {
            return Err(Q3BrowserError::Closed(
                "Q3 browser view operation completed after close",
            ));
        }
        self.host.assert_open();
        Ok(())
    }

    /// Sync a list and return its index (`list`).
    fn sync_list(&mut self, source: i32) -> Result<Option<usize>, Q3BrowserError> {
        self.entry()?;
        if !(0..=3).contains(&source) {
            return Ok(None);
        }
        let index = source as usize;
        let state = self.host.q3_list(source as u8);
        let list = &mut self.lists[index];
        list.pending = state.pending;
        if list.generation != state.generation {
            let resetting = state.reset_generation != list.reset_generation;
            let previous: Vec<Row> = if resetting {
                Vec::new()
            } else {
                list.rows[..list.count as usize].to_vec()
            };
            if resetting {
                list.seen.clear();
                list.overflow.clear();
                for row in list.rows.iter_mut() {
                    row.address = None;
                    row.name.clear();
                    row.ping = 0;
                }
            }
            let mut count = 0usize;
            if source == 2 {
                let addresses: Vec<NetworkAddress> = state.addresses.into_iter().take(8192).collect();
                let keys: HashSet<String> = addresses.iter().map(|address| address_key(address, true)).collect();
                list.seen.retain(|key| keys.contains(key));
                for row in &previous {
                    if let Some(address) = &row.address {
                        if keys.contains(&address_key(address, true)) {
                            list.rows[count] = row.clone();
                            count += 1;
                        }
                    }
                }
                let active: HashSet<String> = list.rows[..count]
                    .iter()
                    .filter_map(|row| row.address.as_ref().map(|address| address_key(address, true)))
                    .collect();
                list.overflow.retain(|address| {
                    keys.contains(&address_key(address, true)) && !active.contains(&address_key(address, true))
                });
                for address in &addresses {
                    let key = address_key(address, true);
                    if list.seen.contains(&key) {
                        continue;
                    }
                    list.seen.insert(key);
                    if count < list.rows.len() {
                        let visible = list.rows[count].visible;
                        list.rows[count] = Row {
                            address: Some(address.clone()),
                            name: String::new(),
                            visible,
                            ping: -1,
                        };
                        count += 1;
                    } else if list.overflow.len() < 4096 {
                        list.overflow.push(address.clone());
                    }
                }
            } else {
                let rows = list.rows.len();
                for address in state.addresses.into_iter().take(rows) {
                    let old = previous
                        .iter()
                        .find(|row| same(row.address.as_ref(), Some(&address)))
                        .cloned();
                    let visible = list.rows[count].visible;
                    list.rows[count] = match old {
                        Some(old) => old,
                        None => Row {
                            address: Some(address),
                            name: String::new(),
                            visible,
                            ping: -1,
                        },
                    };
                    count += 1;
                }
            }
            list.count = count as i32;
            list.generation = state.generation;
            list.reset_generation = state.reset_generation;
        }
        Ok(Some(index))
    }

    /// Clone a row (`row`).
    fn row_clone(&mut self, source: i32, index: i32) -> Result<Option<Row>, Q3BrowserError> {
        let Some(list) = self.sync_list(source)? else {
            return Ok(None);
        };
        if index < 0 {
            return Ok(None);
        }
        Ok(self.lists[list].rows.get(index as usize).cloned())
    }

    /// Clone a row's status (`status`).
    fn status_of(&self, row: &Row) -> Option<ServerStatus> {
        row.address
            .as_ref()
            .and_then(|address| self.core.entry(address))
            .and_then(|entry| entry.status.clone())
    }

    /// Clone a request result.
    fn result_of(&self, handle: DiscoveryRequestHandle) -> Option<DiscoveryRequestResult> {
        self.core.request_result(handle).cloned()
    }

    /// Display name (`name`).
    fn name(&self, row: &Row) -> Result<String, Q3BrowserError> {
        let status = self.status_of(row);
        let fallback = row
            .address
            .as_ref()
            .map(|address| address_key(address, true))
            .unwrap_or_default();
        let status_name = status
            .as_ref()
            .map(|status| status.name.as_str())
            .filter(|name| !name.is_empty());
        let row_name = (!row.name.is_empty()).then_some(row.name.as_str());
        let base = status_name.or(row_name).unwrap_or(&fallback);
        Ok(utf16_slice(&source_command_text(base)?, 31))
    }

    /// Build an info string (`info`).
    fn info(&mut self, pairs: &[(String, String)]) -> Result<String, Q3BrowserError> {
        let mut result = String::new();
        for (key, value) in pairs {
            let mut printed = false;
            let mut forward = |text: &str| {
                (self.print)(text);
                printed = true;
            };
            result = set_info_value(
                &result,
                key,
                value,
                InfoOptions {
                    dialect: Dialect::Q3,
                    maximum_length: 1024,
                    target: InfoTarget::ClientUserinfo,
                    server_high_characters: false,
                },
                &mut forward,
            )?;
            if printed {
                self.entry()?;
            }
        }
        Ok(result)
    }

    /// Server count (`getServerCount`).
    pub fn get_server_count(&mut self, source: i32) -> Result<i32, Q3BrowserError> {
        let Some(list) = self.sync_list(source)? else {
            return Ok(0);
        };
        let list = &self.lists[list];
        Ok(if list.pending { -1 } else { list.count })
    }

    /// Server address string (`getServerAddressString`).
    pub fn get_server_address_string(
        &mut self,
        source: i32,
        index: i32,
        size: usize,
        write: Option<&mut dyn FnMut(&str)>,
    ) -> Result<String, Q3BrowserError> {
        let Some(row) = self.row_clone(source, index)? else {
            return Ok(String::new());
        };
        let text = row
            .address
            .as_ref()
            .map(|address| address_key(address, true))
            .unwrap_or_else(|| "bot".to_owned());
        if let Some(write) = write {
            write(&text);
        }
        copy_string(&text, size)
    }

    /// Server info string (`getServerInfo`).
    pub fn get_server_info(
        &mut self,
        source: i32,
        index: i32,
        size: usize,
        write: Option<&mut dyn FnMut(&str)>,
    ) -> Result<String, Q3BrowserError> {
        let Some(row) = self.row_clone(source, index)? else {
            return Ok(String::new());
        };
        let status = self.status_of(&row);
        let rules = status.as_ref().map(|status| &status.rules);
        let rule = |key: &str| rules.and_then(|rules| rules.get(key)).map(String::as_str).unwrap_or("");
        let text = self.info(
            &[
                ("hostname".to_owned(), self.name(&row)?),
                (
                    "mapname".to_owned(),
                    utf16_slice(
                        &source_command_text(status.as_ref().map(|status| status.map.as_str()).unwrap_or(""))?,
                        31,
                    ),
                ),
                (
                    "clients".to_owned(),
                    status.as_ref().map(|status| status.players).unwrap_or(0).to_string(),
                ),
                (
                    "sv_maxclients".to_owned(),
                    status
                        .as_ref()
                        .map(|status| status.max_players)
                        .unwrap_or(0)
                        .to_string(),
                ),
                ("ping".to_owned(), row.ping.to_string()),
                ("minping".to_owned(), native_atoi(rule("minping"))?.to_string()),
                ("maxping".to_owned(), native_atoi(rule("maxping"))?.to_string()),
                ("game".to_owned(), utf16_slice(&source_command_text(rule("game"))?, 31)),
                ("gametype".to_owned(), native_atoi(rule("gametype"))?.to_string()),
                (
                    "nettype".to_owned(),
                    if row.address.is_none() {
                        "0".to_owned()
                    } else {
                        "1".to_owned()
                    },
                ),
                (
                    "addr".to_owned(),
                    row.address
                        .as_ref()
                        .map(|address| address_key(address, true))
                        .unwrap_or_else(|| "bot".to_owned()),
                ),
                ("punkbuster".to_owned(), native_atoi(rule("punkbuster"))?.to_string()),
            ],
        )?;
        if let Some(write) = write {
            write(&text);
        }
        copy_string(&text, size)
    }

    /// Server ping (`getServerPing`).
    pub fn get_server_ping(&mut self, source: i32, index: i32) -> Result<i32, Q3BrowserError> {
        Ok(self.row_clone(source, index)?.map(|row| row.ping).unwrap_or(-1))
    }

    /// Mark visibility (`markServerVisibleValue`).
    pub fn mark_server_visible_value(&mut self, source: i32, index: i32, visible: i32) -> Result<(), Q3BrowserError> {
        let Some(list) = self.sync_list(source)? else {
            return Ok(());
        };
        let list = &mut self.lists[list];
        if index == -1 {
            for row in list.rows.iter_mut() {
                row.visible = visible;
            }
        } else if index >= 0 {
            if let Some(row) = list.rows.get_mut(index as usize) {
                row.visible = visible;
            }
        }
        Ok(())
    }

    /// Visibility value (`serverVisibilityValue`).
    pub fn server_visibility_value(&mut self, source: i32, index: i32) -> Result<i32, Q3BrowserError> {
        Ok(self.row_clone(source, index)?.map(|row| row.visible).unwrap_or(0))
    }

    /// Reset pings (`resetPings`).
    pub fn reset_pings(&mut self, source: i32) -> Result<(), Q3BrowserError> {
        let Some(list) = self.sync_list(source)? else {
            return Ok(());
        };
        for row in self.lists[list].rows.iter_mut() {
            row.ping = -1;
        }
        Ok(())
    }

    /// Compare servers (`compareServers`).
    pub fn compare_servers(
        &mut self,
        source: i32,
        key: i32,
        direction: i32,
        first: i32,
        second: i32,
    ) -> Result<i32, Q3BrowserError> {
        let (Some(a), Some(b)) = (self.row_clone(source, first)?, self.row_clone(source, second)?) else {
            return Ok(0);
        };
        let compared = if key == 0 {
            compare_text(&self.name(&a)?, &self.name(&b)?)
        } else if key == 1 {
            let left = utf16_slice(
                self.status_of(&a)
                    .as_ref()
                    .map(|status| status.map.as_str())
                    .unwrap_or(""),
                31,
            );
            let right = utf16_slice(
                self.status_of(&b)
                    .as_ref()
                    .map(|status| status.map.as_str())
                    .unwrap_or(""),
                31,
            );
            compare_text(&left, &right)
        } else {
            let value = |row: &Row| -> Result<i64, Q3BrowserError> {
                Ok(if key == 2 {
                    self.status_of(row).map(|status| status.players).unwrap_or(0)
                } else if key == 3 {
                    i64::from(native_atoi(
                        self.status_of(row)
                            .as_ref()
                            .and_then(|status| status.rules.get("gametype"))
                            .map(String::as_str)
                            .unwrap_or(""),
                    )?)
                } else if key == 4 {
                    i64::from(row.ping)
                } else {
                    0
                })
            };
            let (left, right) = (value(&a)?, value(&b)?);
            if left < right {
                -1
            } else if left > right {
                1
            } else {
                0
            }
        };
        Ok(if direction == 0 || compared == 0 {
            compared
        } else {
            -compared
        })
    }

    /// Add a server (`addServer`).
    pub fn add_server(&mut self, source: i32, name: &str, address: &str) -> Result<i32, Q3BrowserError> {
        let Some(list) = self.sync_list(source)? else {
            return Ok(-1);
        };
        if self.lists[list].count >= self.lists[list].rows.len() as i32 {
            return Ok(-1);
        }
        let Some(resolved) = self.host.resolve_q3(address) else {
            return Err(Q3BrowserError::Browser(
                "LAN_AddServer: address resolution failed".to_owned(),
            ));
        };
        let count = self.lists[list].count as usize;
        if self.lists[list].rows[..count]
            .iter()
            .any(|row| same(row.address.as_ref(), Some(&resolved)))
        {
            return Ok(0);
        }
        let row_name = utf16_slice(&source_command_text(name)?, 31);
        let count = self.lists[list].count;
        let row = slot_at_mut(&mut self.lists[list].rows, count)?;
        row.address = Some(resolved.clone());
        row.name = row_name;
        row.visible = 1;
        if !(0..=3).contains(&source) {
            return Ok(-1);
        }
        self.host.add_q3(source as u8, &resolved);
        self.lists[list].count += 1;
        self.lists[list].seen.insert(address_key(&resolved, true));
        let generation = self.host.q3_list(source as u8).generation;
        self.lists[list].generation = generation;
        Ok(1)
    }

    /// Remove a server (`removeServer`).
    pub fn remove_server(&mut self, source: i32, address: &str) -> Result<(), Q3BrowserError> {
        let Some(list) = self.sync_list(source)? else {
            return Ok(());
        };
        let Some(resolved) = self.host.resolve_q3(address) else {
            return Err(Q3BrowserError::Browser(
                "LAN_RemoveServer: address resolution failed".to_owned(),
            ));
        };
        let count = self.lists[list].count as usize;
        let Some(index) = self.lists[list].rows[..count]
            .iter()
            .position(|row| same(row.address.as_ref(), Some(&resolved)))
        else {
            return Ok(());
        };
        if !(0..=3).contains(&source) {
            return Ok(());
        }
        self.host.remove_q3(source as u8, &resolved);
        for next in index..count - 1 {
            let shifted = slot_at(&self.lists[list].rows, next as i32 + 1)?.clone();
            self.lists[list].rows[next] = shifted;
        }
        self.lists[list].count -= 1;
        self.lists[list].seen.remove(&address_key(&resolved, true));
        let generation = self.host.q3_list(source as u8).generation;
        self.lists[list].generation = generation;
        Ok(())
    }

    /// Release a request (`release`).
    fn release(&mut self, handle: Option<DiscoveryRequestHandle>) {
        if let Some(handle) = handle {
            self.core.release_request(handle);
        }
    }

    /// Publish a ping to every row (`publishPing`).
    fn publish_ping(&mut self, address: &NetworkAddress, ping: i32) -> Result<(), Q3BrowserError> {
        for source in 0..4 {
            let Some(list) = self.sync_list(source)? else {
                continue;
            };
            for row in self.lists[list].rows.iter_mut() {
                if same(row.address.as_ref(), Some(address)) {
                    row.ping = ping;
                }
            }
        }
        Ok(())
    }

    /// Ping queue count (`getPingQueueCount`).
    pub fn get_ping_queue_count(&mut self) -> Result<usize, Q3BrowserError> {
        self.entry()?;
        Ok(self.pings.iter().filter(|slot| slot.address.is_some()).count())
    }

    /// Clear a ping slot (`clearPing`).
    pub fn clear_ping(&mut self, index: i32) -> Result<(), Q3BrowserError> {
        self.entry()?;
        if index < 0 {
            return Ok(());
        }
        let Some(slot) = self.pings.get_mut(index as usize) else {
            return Ok(());
        };
        let handle = slot.handle;
        self.release(handle);
        let slot = &mut self.pings[index as usize];
        slot.address = None;
        slot.handle = None;
        Ok(())
    }

    /// Measure a ping slot (`pingTime`).
    fn ping_time(&mut self, index: usize) -> Result<i32, Q3BrowserError> {
        let handle = self.pings[index].handle;
        let result = handle.and_then(|handle| self.result_of(handle));
        if let Some(DiscoveryRequestResult::Completed {
            status,
            ping_milliseconds,
            ..
        }) = result
        {
            let mut text = String::new();
            for (key, value) in &status.rules {
                text.push_str(&format!("\\{key}\\{value}"));
            }
            let mut printed = false;
            let mut forward = |message: &str| {
                (self.print)(message);
                printed = true;
            };
            let info = set_info_value(
                &text,
                "nettype",
                "1",
                InfoOptions {
                    dialect: Dialect::Q3,
                    maximum_length: 1024,
                    target: InfoTarget::ClientUserinfo,
                    server_high_characters: false,
                },
                &mut forward,
            )?;
            self.pings[index].info = info;
            if printed {
                self.entry()?;
            }
            return Ok((ping_milliseconds.trunc() + 1.0) as i32);
        }
        Ok(0)
    }

    /// Ping slot result (`getPing`).
    pub fn get_ping(
        &mut self,
        index: i32,
        size: usize,
        write: Q3PingAddressSink<'_>,
    ) -> Result<Q3PingResult, Q3BrowserError> {
        self.entry()?;
        let slot = slot_at(&self.pings, index)?.clone();
        let Some(address) = slot.address else {
            if let Some(write) = write {
                write(None);
            }
            return Ok(Q3PingResult {
                address: String::new(),
                time: 0,
            });
        };
        let text = address_key(&address, true);
        if let Some(write) = write {
            write(Some(&text));
        }
        let formatted = copy_string(&text, size)?;
        let measured = self.ping_time(index as usize)?;
        let elapsed = ((self.now)() - slot.start).trunc();
        let time = if measured != 0 {
            measured
        } else if elapsed < (self.max_ping)().max(100.0) {
            0
        } else {
            elapsed as i32
        };
        self.publish_ping(&address, measured)?;
        Ok(Q3PingResult {
            address: formatted,
            time,
        })
    }

    /// Ping slot info (`sourcePingInfo`).
    pub fn source_ping_info(
        &mut self,
        index: i32,
        size: usize,
        write: Option<&mut dyn FnMut(&str)>,
    ) -> Result<Option<String>, Q3BrowserError> {
        self.entry()?;
        let slot = slot_at(&self.pings, index)?.clone();
        if slot.address.is_none() {
            return Ok(None);
        }
        self.ping_time(index as usize)?;
        let info = self.pings[index as usize].info.clone();
        if let Some(write) = write {
            write(&info);
        }
        copy_string(&info, size).map(Some)
    }

    /// Start a ping (`startPing`).
    fn start_ping(&mut self, index: usize, address: &NetworkAddress) -> Result<(), Q3BrowserError> {
        let handle = self.pings[index].handle;
        self.release(handle);
        let start = (self.now)();
        let slot = &mut self.pings[index];
        slot.address = Some(address.clone());
        slot.start = start;
        slot.published = false;
        let handle = self
            .core
            .request(address, start, DiscoveryRequestKind::Info, RequestTimeout::Default)?;
        if handle.is_none() {
            self.pings[index].address = None;
            return Err(Q3BrowserError::Browser("Could not send Q3 ping".to_owned()));
        }
        self.pings[index].handle = handle;
        Ok(())
    }

    /// Ping visible rows (`updateVisiblePings`).
    pub fn update_visible_pings(&mut self, source: i32) -> Result<bool, Q3BrowserError> {
        let Some(list) = self.sync_list(source)? else {
            return Ok(false);
        };
        let mut count = self.get_ping_queue_count()?;
        let mut active = false;
        let rows = self.lists[list].count as usize;
        for index in 0..rows {
            if count >= 32 {
                break;
            }
            let row = self.lists[list].rows[index].clone();
            if row.visible == 0 {
                continue;
            }
            if source == 2 && row.ping == 0 {
                let address = self.lists[list].overflow.pop();
                if let Some(address) = address {
                    let row = &mut self.lists[list].rows[index];
                    row.address = Some(address);
                    row.name.clear();
                    row.ping = -1;
                }
                continue;
            }
            if row.ping != -1
                || row.address.is_none()
                || self
                    .pings
                    .iter()
                    .any(|slot| same(slot.address.as_ref(), row.address.as_ref()))
            {
                continue;
            }
            let Some(address) = row.address.clone() else {
                continue;
            };
            let Some(free) = self.pings.iter().position(|slot| slot.address.is_none()) else {
                break;
            };
            self.start_ping(free, &address)?;
            count += 1;
            active = true;
        }
        if count != 0 {
            active = true;
        }
        for index in 0..self.pings.len() as i32 {
            if slot_at(&self.pings, index)?.address.is_some() && self.get_ping(index, 1024, None)?.time != 0 {
                self.clear_ping(index)?;
            }
        }
        Ok(active)
    }

    /// Scan local servers (`localServers`).
    pub fn local_servers(&mut self) -> Result<(), Q3BrowserError> {
        self.entry()?;
        self.host.scan_q3();
        self.sync_list(0)?;
        Ok(())
    }

    /// Fetch masters (`globalServers`).
    pub fn global_servers(
        &mut self,
        source: u8,
        remote: &str,
        protocol: i32,
        keywords: &[String],
    ) -> Result<(), Q3BrowserError> {
        self.entry()?;
        self.host.request_q3_master(source, remote, protocol, keywords);
        self.entry()?;
        self.sync_list(i32::from(source))?;
        Ok(())
    }

    /// Ping an address (`ping`).
    pub fn ping(&mut self, address: &str) -> Result<(), Q3BrowserError> {
        self.entry()?;
        let Some(resolved) = self.host.resolve_q3(address) else {
            return Ok(());
        };
        let now = (self.now)();
        let mut chosen = None;
        for index in 0..self.pings.len() {
            if self.pings[index].address.is_none() {
                chosen = Some(index);
                break;
            }
            let time = self.ping_time(index)?;
            let start = self.pings[index].start;
            if if time == 0 {
                now - start >= 500.0
            } else {
                f64::from(time) >= 500.0
            } {
                chosen = Some(index);
                break;
            }
        }
        let index = chosen.unwrap_or_else(|| {
            let mut oldest = 0;
            for candidate in 1..self.pings.len() {
                if self.pings[candidate].start < self.pings[oldest].start {
                    oldest = candidate;
                }
            }
            oldest
        });
        self.start_ping(index, &resolved)?;
        self.publish_ping(&resolved, 0)?;
        Ok(())
    }

    /// Pick a status slot (`statusSlot`).
    fn status_slot(&self, address: &NetworkAddress) -> usize {
        if let Some(index) = self
            .statuses
            .iter()
            .position(|slot| same(slot.address.as_ref(), Some(address)))
        {
            return index;
        }
        if let Some(index) = self.statuses.iter().position(|slot| slot.retrieved) {
            return index;
        }
        let mut oldest = 0;
        for candidate in 1..self.statuses.len() {
            if self.statuses[candidate].start < self.statuses[oldest].start {
                oldest = candidate;
            }
        }
        oldest
    }

    /// Format a status (`statusText`).
    fn status_text(status: &ServerStatus) -> String {
        let mut text = String::new();
        for (key, value) in &status.rules {
            text.push_str(&format!("\\{key}\\{value}"));
        }
        text = utf16_slice(&text, 8191) + "\\";
        for player in &status.player_details {
            text = utf16_slice(
                &format!("{text}\\{} {} \"{}\"", player.score, player.ping, player.name),
                8191,
            );
        }
        utf16_slice(&format!("{text}\\"), 8191)
    }

    /// Start a status request (`startStatus`).
    fn start_status(&mut self, index: usize, address: &NetworkAddress) -> Result<(), Q3BrowserError> {
        let handle = self.statuses[index].handle;
        self.release(handle);
        let start = (self.now)();
        let slot = &mut self.statuses[index];
        slot.address = Some(address.clone());
        slot.start = start;
        slot.retrieved = false;
        let handle = self
            .core
            .request(address, start, DiscoveryRequestKind::Status, RequestTimeout::Default)?;
        if handle.is_none() {
            self.statuses[index].retrieved = true;
            return Err(Q3BrowserError::Browser(
                "Could not send Q3 server status request".to_owned(),
            ));
        }
        self.statuses[index].handle = handle;
        Ok(())
    }

    /// Server status (`serverStatus`).
    pub fn server_status(
        &mut self,
        address: Option<&str>,
        size: Option<usize>,
        write: Option<&mut dyn FnMut(&str)>,
    ) -> Result<Option<String>, Q3BrowserError> {
        self.entry()?;
        let Some(address) = address else {
            let handles: Vec<Option<DiscoveryRequestHandle>> = self.statuses.iter().map(|slot| slot.handle).collect();
            for handle in handles {
                self.release(handle);
            }
            for slot in self.statuses.iter_mut() {
                slot.handle = None;
                slot.address = None;
                slot.retrieved = true;
                slot.print = false;
            }
            return Ok(None);
        };
        let Some(resolved) = self.host.resolve_q3(address) else {
            return Ok(None);
        };
        let Some(size) = size else {
            let found = self
                .statuses
                .iter()
                .position(|slot| same(slot.address.as_ref(), Some(&resolved)));
            if let Some(index) = found {
                let handle = self.statuses[index].handle;
                self.release(handle);
                let slot = &mut self.statuses[index];
                slot.handle = None;
                slot.address = None;
                slot.retrieved = true;
                slot.print = false;
            }
            return Ok(None);
        };
        let index = self.status_slot(&resolved);
        if !same(self.statuses[index].address.as_ref(), Some(&resolved)) && !self.statuses[index].retrieved {
            return Ok(None);
        }
        let handle = self.statuses[index].handle;
        let result = handle.and_then(|handle| self.result_of(handle));
        if same(self.statuses[index].address.as_ref(), Some(&resolved))
            && matches!(result, Some(DiscoveryRequestResult::Completed { .. }))
        {
            let Some(DiscoveryRequestResult::Completed { status, .. }) = result else {
                return Ok(None);
            };
            let text = Self::status_text(&status);
            if let Some(write) = write {
                write(&text);
            }
            let value = copy_string(&text, size)?;
            self.statuses[index].retrieved = true;
            self.statuses[index].start = 0.0;
            return Ok(Some(value));
        }
        let resend = (self.status_resend_time)();
        let now = (self.now)();
        if !same(self.statuses[index].address.as_ref(), Some(&resolved))
            || self.statuses[index].handle.is_none()
            || self.statuses[index].start < now - resend
        {
            self.statuses[index].print = false;
            self.start_status(index, &resolved)?;
        }
        Ok(None)
    }

    /// Print a server status (`serverStatusCommand`).
    pub fn server_status_command(&mut self, address: &str) -> Result<(), Q3BrowserError> {
        self.entry()?;
        let Some(resolved) = self.host.resolve_q3(address) else {
            return Ok(());
        };
        let index = self.status_slot(&resolved);
        self.start_status(index, &resolved)?;
        self.statuses[index].print = true;
        Ok(())
    }

    /// Poll requests (`poll`).
    pub fn poll(&mut self) -> Result<(), Q3BrowserError> {
        self.entry()?;
        for index in 0..self.pings.len() {
            if self.pings[index].address.is_none() || self.pings[index].published {
                continue;
            }
            let time = self.ping_time(index)?;
            if time != 0 {
                let Some(address) = self.pings[index].address.clone() else {
                    continue;
                };
                self.publish_ping(&address, time)?;
                self.pings[index].published = true;
            }
        }
        for index in 0..self.statuses.len() {
            if !self.statuses[index].print || self.statuses[index].handle.is_none() {
                continue;
            }
            let Some(handle) = self.statuses[index].handle else {
                continue;
            };
            let result = self.result_of(handle);
            let Some(DiscoveryRequestResult::Completed { status, .. }) = result else {
                continue;
            };
            (self.print)("Server settings:\n");
            for (key, value) in &status.rules {
                (self.print)(&format!("{}{value}\n", pad_end(key, 24)));
            }
            (self.print)("\nPlayers:\nnum: score: ping: name:\n");
            for (player_index, player) in status.player_details.iter().enumerate() {
                (self.print)(&format!(
                    "{}   {}    {}   \"{}\"\n",
                    pad_end(&player_index.to_string(), 2),
                    pad_end(&player.score.to_string(), 3),
                    pad_end(&player.ping.to_string(), 3),
                    player.name
                ));
            }
            self.statuses[index].print = false;
            self.statuses[index].retrieved = true;
            self.entry()?;
        }
        Ok(())
    }

    /// Load cached servers (`loadCachedServers`).
    pub fn load_cached_servers(&mut self) -> Result<(), Q3BrowserError> {
        self.entry()?;
        let cache = self.host.load_q3_cache();
        for source in [1u8, 2, 3] {
            let Some(list) = self.sync_list(i32::from(source))? else {
                continue;
            };
            let saved = cache
                .as_ref()
                .and_then(|cache| cache.lists.iter().find(|value| value.source == source));
            let Some(saved) = saved else {
                continue;
            };
            if source == 2 {
                let canonical = self.host.q3_list(source).addresses;
                let keys: HashSet<String> = canonical.iter().map(|address| address_key(address, true)).collect();
                let selected: Vec<&Q3BrowserCacheRow> = saved
                    .rows
                    .iter()
                    .filter(|row| keys.contains(&address_key(&row.address, true)))
                    .collect();
                let count = selected.len().min(self.lists[list].rows.len());
                self.lists[list].count = count as i32;
                for (index, row) in self.lists[list].rows.iter_mut().enumerate() {
                    *row = selected.get(index).map_or_else(Row::empty, |saved| Row {
                        address: Some(saved.address.clone()),
                        name: saved.name.clone(),
                        visible: saved.visible,
                        ping: saved.ping,
                    });
                }
                self.lists[list].seen.clear();
                for address in &canonical {
                    self.lists[list].seen.insert(address_key(address, true));
                }
                self.lists[list].overflow.clear();
            } else {
                let count = self.lists[list].count;
                for index in 0..self.lists[list].rows.len() {
                    let current = self.lists[list].rows[index].clone();
                    let replacement = if (index as i32) < count {
                        saved
                            .rows
                            .iter()
                            .find(|row| same(Some(&row.address), current.address.as_ref()))
                    } else {
                        None
                    };
                    self.lists[list].rows[index] = match (replacement, (index as i32) < count) {
                        (Some(saved), _) => Row {
                            address: Some(saved.address.clone()),
                            name: saved.name.clone(),
                            visible: saved.visible,
                            ping: saved.ping,
                        },
                        (None, true) => current,
                        (None, false) => Row::empty(),
                    };
                }
            }
        }
        Ok(())
    }

    /// Save servers to cache (`saveServersToCache`).
    pub fn save_servers_to_cache(&mut self) -> Result<(), Q3BrowserError> {
        self.entry()?;
        let mut lists = Vec::new();
        for source in [1u8, 2, 3] {
            let Some(list) = self.sync_list(i32::from(source))? else {
                continue;
            };
            let mut rows = Vec::new();
            for row in self.lists[list].rows.iter().take(self.lists[list].count as usize) {
                if let Some(address) = &row.address {
                    rows.push(Q3BrowserCacheRow {
                        address: address.clone(),
                        name: row.name.clone(),
                        visible: row.visible,
                        ping: row.ping,
                    });
                }
            }
            lists.push(Q3BrowserCacheList { source, rows });
        }
        self.host.save_q3_cache(&Q3BrowserCacheView { lists });
        self.entry()?;
        Ok(())
    }

    /// Close the view (`close`).
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let handles: Vec<Option<DiscoveryRequestHandle>> = self
            .pings
            .iter()
            .map(|slot| slot.handle)
            .chain(self.statuses.iter().map(|slot| slot.handle))
            .collect();
        for handle in handles {
            self.release(handle);
        }
        for slot in self.pings.iter_mut() {
            slot.handle = None;
            slot.address = None;
        }
        for slot in self.statuses.iter_mut() {
            slot.handle = None;
            slot.address = None;
        }
    }
}

/// Mutable slot indexing (`slotAt`).
fn slot_at_mut<T>(slots: &mut [T], index: i32) -> Result<&mut T, Q3BrowserError> {
    if index < 0 {
        return Err(Q3BrowserError::Browser(format!(
            "Server browser source index {index} outside {}",
            slots.len()
        )));
    }
    let len = slots.len();
    slots
        .get_mut(index as usize)
        .ok_or_else(|| Q3BrowserError::Browser(format!("Server browser source index {index} outside {len}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::endpoint::ipv4_address;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, HashMap};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    struct FixtureState {
        lists: HashMap<u8, Q3BrowserListState>,
        cache: Option<Q3BrowserCacheView>,
        saved: Vec<Q3BrowserCacheView>,
        masters: Vec<(u8, String, i32, Vec<String>)>,
        scans: usize,
    }

    struct Fixture {
        state: Rc<RefCell<FixtureState>>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                state: Rc::new(RefCell::new(FixtureState {
                    lists: HashMap::new(),
                    cache: None,
                    saved: Vec::new(),
                    masters: Vec::new(),
                    scans: 0,
                })),
            }
        }

        fn spy(&self) -> Rc<RefCell<FixtureState>> {
            self.state.clone()
        }

        fn with_list(self, source: u8, addresses: Vec<NetworkAddress>) -> Self {
            self.state.borrow_mut().lists.insert(
                source,
                Q3BrowserListState {
                    addresses,
                    pending: false,
                    generation: 1,
                    reset_generation: 0,
                },
            );
            self
        }
    }

    impl Q3BrowserHost for Fixture {
        fn q3_list(&mut self, source: Q3BrowserSource) -> Q3BrowserListState {
            self.state
                .borrow()
                .lists
                .get(&source)
                .cloned()
                .unwrap_or(Q3BrowserListState {
                    addresses: Vec::new(),
                    pending: false,
                    generation: 0,
                    reset_generation: 0,
                })
        }

        fn resolve_q3(&mut self, text: &str) -> Option<NetworkAddress> {
            let (host, port) = text.split_once(':')?;
            let octets: Vec<u8> = host.split('.').map(|part| part.parse().unwrap_or(0)).collect();
            if octets.len() != 4 {
                return None;
            }
            ipv4_address(
                [octets[0], octets[1], octets[2], octets[3]],
                port.parse().unwrap_or(0),
                true,
            )
            .ok()
        }

        fn add_q3(&mut self, source: Q3BrowserSource, address: &NetworkAddress) {
            let mut state = self.state.borrow_mut();
            let list = state.lists.entry(source).or_insert(Q3BrowserListState {
                addresses: Vec::new(),
                pending: false,
                generation: 0,
                reset_generation: 0,
            });
            list.addresses.push(address.clone());
            list.generation += 1;
        }

        fn remove_q3(&mut self, source: Q3BrowserSource, address: &NetworkAddress) {
            let address = address.clone();
            if let Some(list) = self.state.borrow_mut().lists.get_mut(&source) {
                list.addresses
                    .retain(|candidate| address_key(candidate, true) != address_key(&address, true));
                list.generation += 1;
            }
        }

        fn scan_q3(&mut self) {
            self.state.borrow_mut().scans += 1;
        }

        fn request_q3_master(&mut self, source: u8, remote: &str, protocol: i32, keywords: &[String]) {
            self.state
                .borrow_mut()
                .masters
                .push((source, remote.to_owned(), protocol, keywords.to_vec()));
        }

        fn load_q3_cache(&mut self) -> Option<Q3BrowserCacheView> {
            self.state.borrow().cache.clone()
        }

        fn save_q3_cache(&mut self, view: &Q3BrowserCacheView) {
            self.state.borrow_mut().saved.push(view.clone());
        }

        fn assert_open(&mut self) {}
    }

    struct FakeWire;

    impl crate::services::discovery::DiscoveryWire for FakeWire {
        fn query(&self, _kind: DiscoveryRequestKind, _challenge: &str) -> Result<Vec<u8>, DiscoveryError> {
            Ok(vec![1, 2, 3])
        }

        fn master_query(&self) -> Result<Vec<u8>, DiscoveryError> {
            Ok(Vec::new())
        }

        fn heartbeat(&self, _active: bool) -> Result<Vec<u8>, DiscoveryError> {
            Ok(Vec::new())
        }
    }

    struct FakeTransport {
        sent: Vec<(NetworkAddress, Vec<u8>)>,
    }

    impl crate::services::discovery::PacketSender for FakeTransport {
        fn send(&mut self, to: &NetworkAddress, bytes: &[u8]) -> bool {
            self.sent.push((to.clone(), bytes.to_vec()));
            true
        }
    }

    fn address(byte: u8) -> NetworkAddress {
        ipv4_address([10, 0, 0, byte], 27960, false).unwrap()
    }

    fn status(name: &str) -> ServerStatus {
        ServerStatus {
            name: name.to_owned(),
            map: "q3dm1".to_owned(),
            players: 2,
            max_players: 8,
            rules: BTreeMap::from([("game".to_owned(), "baseq3".to_owned())]),
            player_details: Vec::new(),
            wire: crate::common::session::WireSelection::Source {
                protocol: crate::protocol::ProtocolIdentity::Q3,
            },
        }
    }

    #[test]
    fn lists_count_and_format_servers() {
        let wire = FakeWire;
        let mut transport = FakeTransport { sent: Vec::new() };
        let mut core = ServerBrowser::new(&wire, &mut transport);
        let mut host = Fixture::new().with_list(1, vec![address(1), address(2)]);
        let mut view = Q3BrowserView::new(&mut core, &mut host, || 1000.0, || 800.0, || 500.0, |_| {});
        assert_eq!(view.get_server_count(1).unwrap(), 2);
        assert_eq!(view.get_server_count(9).unwrap(), 0);
        let text = view.get_server_address_string(1, 0, 64, None).unwrap();
        assert!(text.starts_with("10.0.0.1"), "{text}");
        assert_eq!(view.get_server_ping(1, 0).unwrap(), -1);
        assert_eq!(view.get_server_ping(1, 9).unwrap(), 0);
        assert_eq!(view.get_server_ping(1, 128).unwrap(), -1);
    }

    #[test]
    fn info_strings_carry_browser_rules() {
        let wire = FakeWire;
        let mut transport = FakeTransport { sent: Vec::new() };
        let mut core = ServerBrowser::new(&wire, &mut transport);
        core.add(address(1), crate::services::discovery::DiscoverySource::Direct, 0.0);
        core.request(&address(1), 0.0, DiscoveryRequestKind::Info, RequestTimeout::Default)
            .unwrap();
        assert!(core.receive(&address(1), status("Frag House"), None, 25.0, None));
        let mut host = Fixture::new().with_list(1, vec![address(1)]);
        let mut view = Q3BrowserView::new(&mut core, &mut host, || 1000.0, || 800.0, || 500.0, |_| {});
        let info = view.get_server_info(1, 0, 1024, None).unwrap();
        assert!(info.contains("Frag House"), "{info}");
        assert!(info.contains("\\mapname\\q3dm1"), "{info}");
        assert!(info.contains("\\clients\\2"), "{info}");
        assert_eq!(view.compare_servers(1, 4, 0, 0, 0).unwrap(), 0);
    }

    #[test]
    fn servers_add_and_remove() {
        let wire = FakeWire;
        let mut transport = FakeTransport { sent: Vec::new() };
        let mut core = ServerBrowser::new(&wire, &mut transport);
        let mut host = Fixture::new().with_list(3, Vec::new());
        let mut view = Q3BrowserView::new(&mut core, &mut host, || 1000.0, || 800.0, || 500.0, |_| {});
        assert_eq!(view.add_server(3, "home", "10.0.0.9:27960").unwrap(), 1);
        assert_eq!(view.add_server(3, "home", "10.0.0.9:27960").unwrap(), 0);
        assert_eq!(view.get_server_count(3).unwrap(), 1);
        view.remove_server(3, "10.0.0.9:27960").unwrap();
        assert_eq!(view.get_server_count(3).unwrap(), 0);
        assert!(view.add_server(3, "x", "bogus").is_err());
    }

    #[test]
    fn pings_measure_and_publish() {
        let wire = FakeWire;
        let mut transport = FakeTransport { sent: Vec::new() };
        let mut core = ServerBrowser::new(&wire, &mut transport);
        let mut host = Fixture::new().with_list(1, vec![address(5)]);
        let clock = Arc::new(Mutex::new(1000.0f64));
        let tick = clock.clone();
        let mut view = Q3BrowserView::new(
            &mut core,
            &mut host,
            move || *tick.lock().unwrap(),
            || 800.0,
            || 500.0,
            |_| {},
        );
        view.mark_server_visible_value(1, 0, 1).unwrap();
        assert!(view.update_visible_pings(1).unwrap());
        assert_eq!(view.get_ping_queue_count().unwrap(), 1);
        *clock.lock().unwrap() = 5000.0;
        let ping = view.get_ping(0, 64, None).unwrap();
        assert!(ping.time > 0);
        assert_eq!(view.get_server_ping(1, 0).unwrap(), 0);
    }

    #[test]
    fn caches_round_trip() {
        let wire = FakeWire;
        let mut transport = FakeTransport { sent: Vec::new() };
        let mut core = ServerBrowser::new(&wire, &mut transport);
        let mut host = Fixture::new().with_list(1, vec![address(7)]);
        let spy = host.spy();
        spy.borrow_mut().cache = Some(Q3BrowserCacheView {
            lists: vec![Q3BrowserCacheList {
                source: 1,
                rows: vec![Q3BrowserCacheRow {
                    address: address(7),
                    name: "cached".to_owned(),
                    visible: 1,
                    ping: 42,
                }],
            }],
        });
        let mut view = Q3BrowserView::new(&mut core, &mut host, || 1000.0, || 800.0, || 500.0, |_| {});
        view.load_cached_servers().unwrap();
        assert_eq!(view.server_visibility_value(1, 0).unwrap(), 1);
        assert_eq!(view.get_server_ping(1, 0).unwrap(), 42);
        view.save_servers_to_cache().unwrap();
        assert_eq!(spy.borrow().saved.len(), 1);
        assert_eq!(spy.borrow().saved[0].lists.len(), 3);
    }

    #[test]
    fn masters_and_scans_delegate() {
        let wire = FakeWire;
        let mut transport = FakeTransport { sent: Vec::new() };
        let mut core = ServerBrowser::new(&wire, &mut transport);
        let mut host = Fixture::new();
        let spy = host.spy();
        let mut view = Q3BrowserView::new(&mut core, &mut host, || 1000.0, || 800.0, || 500.0, |_| {});
        view.global_servers(1, "master.example", 68, &["ctf".to_owned()])
            .unwrap();
        view.local_servers().unwrap();
        assert_eq!(spy.borrow().masters.len(), 1);
        assert_eq!(spy.borrow().scans, 1);
        view.global_servers(3, "x", 68, &[]).unwrap();
        assert_eq!(spy.borrow().masters.len(), 2);
    }

    #[test]
    fn close_ends_operations() {
        let wire = FakeWire;
        let mut transport = FakeTransport { sent: Vec::new() };
        let mut core = ServerBrowser::new(&wire, &mut transport);
        let mut host = Fixture::new();
        let mut view = Q3BrowserView::new(&mut core, &mut host, || 1000.0, || 800.0, || 500.0, |_| {});
        view.close();
        assert_eq!(
            view.get_server_count(1),
            Err(Q3BrowserError::Closed(
                "Q3 browser view operation completed after close"
            ))
        );
        assert_eq!(
            view.get_ping_queue_count(),
            Err(Q3BrowserError::Closed(
                "Q3 browser view operation completed after close"
            ))
        );
    }

    #[test]
    fn text_helpers_match_donor() {
        assert_eq!(compare_text("abc", "ABD"), -1);
        assert_eq!(compare_text("b", "a"), 1);
        assert_eq!(compare_text("Q3", "q3"), 0);
        assert_eq!(copy_string("hello", 4).unwrap(), "hel");
        assert_eq!(
            copy_string("hello", 0),
            Err(Q3BrowserError::Fatal("Q_strncpyz: destsize < 1"))
        );
        assert_eq!(pad_end("ab", 4), "ab  ");
    }
}
