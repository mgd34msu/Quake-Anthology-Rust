//! UI server-browser (LAN) traps.
//!
//! Provenance: `src/compat/qvm/client-browser-syscalls.ts` (UI LAN traps
//! from id Software `cl_ui.c`/`cl_main.c`). [`BrowserHost`] is a local
//! mirror of `Q3BrowserView`; donor promises become direct returns, and the
//! donor's write-callback style becomes owned `Option<String>` results that
//! the bridge copies into guest memory.

use super::client_state::{CallKind, HostCall, QvmRole, SyscallMemory};
use crate::error::GuestError;

/// Ping query result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingResult {
    /// Ping time in milliseconds.
    pub time: i32,
    /// Server address, if the slot holds one.
    pub address: Option<String>,
}

/// Host server-browser view.
pub trait BrowserHost {
    /// Number of queued pings.
    fn get_ping_queue_count(&mut self) -> i32;
    /// Clear a ping slot.
    fn clear_ping(&mut self, index: i32);
    /// Ping slot contents.
    fn get_ping(&mut self, index: i32) -> PingResult;
    /// Ping info text for a slot.
    fn source_ping_info(&mut self, index: i32) -> Option<String>;
    /// Server count for a source.
    fn get_server_count(&mut self, source: i32) -> i32;
    /// Server address string, if the slot holds one.
    fn get_server_address_string(&mut self, source: i32, index: i32) -> Option<String>;
    /// Server info string, if the slot holds one.
    fn get_server_info(&mut self, source: i32, index: i32) -> Option<String>;
    /// Mark a server's visibility value.
    fn mark_server_visible(&mut self, source: i32, index: i32, visible: i32);
    /// Update visible pings.
    fn update_visible_pings(&mut self, source: i32) -> bool;
    /// Reset pings for a source.
    fn reset_pings(&mut self, source: i32);
    /// Load cached servers.
    fn load_cached_servers(&mut self);
    /// Save servers to cache.
    fn save_servers_to_cache(&mut self);
    /// Add a server, returning its index.
    fn add_server(&mut self, source: i32, name: &str, address: &str) -> i32;
    /// Remove a server.
    fn remove_server(&mut self, source: i32, address: &str);
    /// Query server status text.
    fn server_status(&mut self, address: Option<&str>) -> Option<String>;
    /// Server ping value.
    fn get_server_ping(&mut self, source: i32, index: i32) -> i32;
    /// Server visibility value.
    fn server_visibility(&mut self, source: i32, index: i32) -> i32;
    /// Compare two servers for sorting.
    fn compare_servers(&mut self, source: i32, key: i32, direction: i32, first: i32, second: i32) -> i32;
}

fn has_server_record(source: i32, index: i32) -> bool {
    index >= 0 && (source == 2 && index < 4096 || matches!(source, 0 | 1 | 3) && index < 128)
}

/// Copy a server name with the donor's 31-byte `LAN_AddServer` limit.
fn read_server_name(memory: &SyscallMemory, word: i32) -> Result<String, GuestError> {
    let base = memory.pointer(word).ok_or_else(|| GuestError::invalid("Q_strncpyz: NULL src"))?;
    let mut name = String::new();
    for offset in 0..31 {
        let byte = memory.get(base + offset).map_err(|_| {
            GuestError::invalid("Server name copy exceeds QVM allocation")
        })?;
        if byte == 0 {
            break;
        }
        name.push(char::from(byte));
    }
    Ok(name)
}

/// Dispatch a browser trap. Returns `Ok(None)` when unhandled.
pub fn client_browser_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    browser: &mut dyn BrowserHost,
) -> Result<Option<i32>, GuestError> {
    if call.role != QvmRole::Ui {
        return Ok(None);
    }
    if call.kind == CallKind::Extension
        && call.abi_profile == super::client_state::AbiProfile::Legacy
        && (46..=49).contains(&call.code)
    {
        let source = if call.code <= 47 { 0 } else { 2 };
        if call.code == 46 || call.code == 48 {
            return Ok(Some(browser.get_server_count(source)));
        }
        let index = call.int(1)?;
        let word = call.int(2)?;
        let capacity = call.int(3)?;
        if capacity != 0 {
            let range = memory.span(word, 1, 0)?;
            memory.set(range.start, 0)?;
        }
        if let Some(text) = browser.get_server_address_string(source, index) {
            memory.write_string(word, &text, capacity as usize)?;
        }
        return Ok(Some(0));
    }
    if call.kind != CallKind::Engine {
        return Ok(None);
    }
    match call.code {
        46 => Ok(Some(browser.get_ping_queue_count())),
        47 => {
            browser.clear_ping(call.int(1)?);
            Ok(Some(0))
        }
        48 => {
            let index = call.int(1)?;
            let word = call.int(2)?;
            let capacity = call.int(3)?;
            let time_word = call.int(4)?;
            let ping = browser.get_ping(index);
            match ping.address {
                None => {
                    let range = memory.span(word, 1, 0)?;
                    memory.set(range.start, 0)?;
                }
                Some(address) => memory.write_string(word, &address, capacity as usize)?,
            }
            memory.span(time_word, 4, 0)?;
            let base = memory.pointer(time_word).expect("checked span");
            memory.write_i32(base, ping.time)?;
            Ok(Some(0))
        }
        49 => {
            let index = call.int(1)?;
            let word = call.int(2)?;
            let capacity = call.int(3)?;
            match browser.source_ping_info(index) {
                None => {
                    if capacity != 0 {
                        let range = memory.span(word, 1, 0)?;
                        memory.set(range.start, 0)?;
                    }
                }
                Some(info) => memory.write_string(word, &info, capacity as usize)?,
            }
            Ok(Some(0))
        }
        65 => Ok(Some(browser.get_server_count(call.int(1)?))),
        66 | 67 => {
            let source = call.int(1)?;
            let index = call.int(2)?;
            let word = call.int(3)?;
            let capacity = call.int(4)?;
            if call.code == 67 && word == 0 {
                return Ok(Some(0));
            }
            if call.code == 67 || !has_server_record(source, index) {
                let range = memory.span(word, 1, 0)?;
                memory.set(range.start, 0)?;
            }
            if !has_server_record(source, index) {
                return Ok(Some(0));
            }
            let text = if call.code == 66 {
                browser.get_server_address_string(source, index)
            } else {
                browser.get_server_info(source, index)
            };
            if let Some(text) = text {
                memory.write_string(word, &text, capacity as usize)?;
            }
            Ok(Some(0))
        }
        68 => {
            browser.mark_server_visible(call.int(1)?, call.int(2)?, call.int(3)?);
            Ok(Some(0))
        }
        69 => Ok(Some(i32::from(browser.update_visible_pings(call.int(1)?)))),
        70 => {
            browser.reset_pings(call.int(1)?);
            Ok(Some(0))
        }
        71 => {
            browser.load_cached_servers();
            Ok(Some(0))
        }
        72 => {
            browser.save_servers_to_cache();
            Ok(Some(0))
        }
        73 => {
            let source = call.int(1)?;
            let name = read_server_name(memory, call.int(2)?)?;
            let address = memory.read_string(call.int(3)?)?;
            Ok(Some(browser.add_server(source, &name, &address)))
        }
        74 => {
            let source = call.int(1)?;
            let address = memory.read_string(call.int(2)?)?;
            browser.remove_server(source, &address);
            Ok(Some(0))
        }
        82 => {
            let address_word = call.int(1)?;
            let word = call.int(2)?;
            let capacity = call.int(3)?;
            let address = if address_word == 0 { None } else { Some(memory.read_string(address_word)?) };
            let status = browser.server_status(address.as_deref());
            if word != 0 {
                if let Some(text) = &status {
                    memory.write_string(word, text, capacity as usize)?;
                }
            }
            Ok(Some(i32::from(status.is_some())))
        }
        83 => Ok(Some(browser.get_server_ping(call.int(1)?, call.int(2)?))),
        84 => Ok(Some(browser.server_visibility(call.int(1)?, call.int(2)?))),
        85 => Ok(Some(browser.compare_servers(call.int(1)?, call.int(2)?, call.int(3)?, call.int(4)?, call.int(5)?))),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_state::AbiProfile;
    use super::*;

    struct FakeBrowser {
        log: Vec<String>,
    }

    impl BrowserHost for FakeBrowser {
        fn get_ping_queue_count(&mut self) -> i32 {
            3
        }
        fn clear_ping(&mut self, index: i32) {
            self.log.push(format!("clear {index}"));
        }
        fn get_ping(&mut self, index: i32) -> PingResult {
            if index == 0 {
                PingResult { time: 42, address: Some("1.2.3.4:27960".to_string()) }
            } else {
                PingResult { time: -1, address: None }
            }
        }
        fn source_ping_info(&mut self, index: i32) -> Option<String> {
            (index == 0).then(|| "info".to_string())
        }
        fn get_server_count(&mut self, source: i32) -> i32 {
            10 + source
        }
        fn get_server_address_string(&mut self, _source: i32, index: i32) -> Option<String> {
            (index == 0).then(|| "5.6.7.8:27960".to_string())
        }
        fn get_server_info(&mut self, _source: i32, index: i32) -> Option<String> {
            (index == 0).then(|| "\\map\\q3dm1".to_string())
        }
        fn mark_server_visible(&mut self, source: i32, index: i32, visible: i32) {
            self.log.push(format!("visible {source} {index} {visible}"));
        }
        fn update_visible_pings(&mut self, _source: i32) -> bool {
            true
        }
        fn reset_pings(&mut self, source: i32) {
            self.log.push(format!("reset {source}"));
        }
        fn load_cached_servers(&mut self) {
            self.log.push("load".to_string());
        }
        fn save_servers_to_cache(&mut self) {
            self.log.push("save".to_string());
        }
        fn add_server(&mut self, source: i32, name: &str, address: &str) -> i32 {
            self.log.push(format!("add {source} {name} {address}"));
            4
        }
        fn remove_server(&mut self, source: i32, address: &str) {
            self.log.push(format!("remove {source} {address}"));
        }
        fn server_status(&mut self, address: Option<&str>) -> Option<String> {
            address.map(|value| format!("status {value}"))
        }
        fn get_server_ping(&mut self, _source: i32, _index: i32) -> i32 {
            50
        }
        fn server_visibility(&mut self, _source: i32, _index: i32) -> i32 {
            1
        }
        fn compare_servers(&mut self, _source: i32, _key: i32, _direction: i32, first: i32, second: i32) -> i32 {
            first - second
        }
    }

    fn ui(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Ui, code, args, AbiProfile::Modern)
    }

    #[test]
    fn ping_traps() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut browser = FakeBrowser { log: Vec::new() };
        assert_eq!(client_browser_syscall(&ui(46, &[]), &mut memory, &mut browser).unwrap(), Some(3));
        assert_eq!(client_browser_syscall(&ui(47, &[2]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(
            client_browser_syscall(&ui(48, &[0, 256, 64, 512]), &mut memory, &mut browser).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(256).unwrap(), "1.2.3.4:27960");
        assert_eq!(memory.read_i32(512).unwrap(), 42);
        assert_eq!(
            client_browser_syscall(&ui(48, &[1, 256, 64, 512]), &mut memory, &mut browser).unwrap(),
            Some(0)
        );
        assert_eq!(memory.get(256).unwrap(), 0);
        assert_eq!(client_browser_syscall(&ui(49, &[0, 256, 64]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(memory.read_string(256).unwrap(), "info");
        assert_eq!(client_browser_syscall(&ui(49, &[1, 256, 64]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(memory.get(256).unwrap(), 0);
        assert_eq!(browser.log, vec!["clear 2".to_string()]);
    }

    #[test]
    fn server_address_and_info() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut browser = FakeBrowser { log: Vec::new() };
        assert_eq!(client_browser_syscall(&ui(65, &[0]), &mut memory, &mut browser).unwrap(), Some(10));
        assert_eq!(client_browser_syscall(&ui(66, &[0, 0, 256, 64]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(memory.read_string(256).unwrap(), "5.6.7.8:27960");
        assert_eq!(client_browser_syscall(&ui(67, &[0, 0, 256, 64]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(memory.read_string(256).unwrap(), "\\map\\q3dm1");
        assert_eq!(client_browser_syscall(&ui(66, &[0, 200, 256, 64]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(memory.get(256).unwrap(), 0);
        assert_eq!(client_browser_syscall(&ui(67, &[0, 0, 0, 64]), &mut memory, &mut browser).unwrap(), Some(0));
    }

    #[test]
    fn visible_pings_and_cache() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut browser = FakeBrowser { log: Vec::new() };
        assert_eq!(client_browser_syscall(&ui(68, &[0, 1, 1]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(client_browser_syscall(&ui(69, &[0]), &mut memory, &mut browser).unwrap(), Some(1));
        assert_eq!(client_browser_syscall(&ui(70, &[0]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(client_browser_syscall(&ui(71, &[]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(client_browser_syscall(&ui(72, &[]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(browser.log, vec!["visible 0 1 1".to_string(), "reset 0".to_string(), "load".to_string(), "save".to_string()]);
    }

    #[test]
    fn add_and_remove_server() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(256, "my-server", 10).unwrap();
        memory.write_string(512, "9.9.9.9:27960", 14).unwrap();
        let mut browser = FakeBrowser { log: Vec::new() };
        assert_eq!(client_browser_syscall(&ui(73, &[0, 256, 512]), &mut memory, &mut browser).unwrap(), Some(4));
        assert_eq!(client_browser_syscall(&ui(74, &[0, 512]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(
            browser.log,
            vec![
                "add 0 my-server 9.9.9.9:27960".to_string(),
                "remove 0 9.9.9.9:27960".to_string(),
            ]
        );
    }

    #[test]
    fn server_name_truncates_at_31_bytes() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let long = "a".repeat(40);
        memory.write_string(256, &long, 41).unwrap();
        memory.write_string(512, "addr", 5).unwrap();
        let mut browser = FakeBrowser { log: Vec::new() };
        assert_eq!(client_browser_syscall(&ui(73, &[0, 256, 512]), &mut memory, &mut browser).unwrap(), Some(4));
        assert_eq!(browser.log[0], format!("add 0 {} addr", "a".repeat(31)));
    }

    #[test]
    fn status_ping_visibility_compare() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(256, "7.7.7.7", 8).unwrap();
        let mut browser = FakeBrowser { log: Vec::new() };
        assert_eq!(client_browser_syscall(&ui(82, &[256, 512, 64]), &mut memory, &mut browser).unwrap(), Some(1));
        assert_eq!(memory.read_string(512).unwrap(), "status 7.7.7.7");
        assert_eq!(client_browser_syscall(&ui(82, &[0, 512, 64]), &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(client_browser_syscall(&ui(83, &[0, 1]), &mut memory, &mut browser).unwrap(), Some(50));
        assert_eq!(client_browser_syscall(&ui(84, &[0, 1]), &mut memory, &mut browser).unwrap(), Some(1));
        assert_eq!(client_browser_syscall(&ui(85, &[0, 0, 0, 5, 3]), &mut memory, &mut browser).unwrap(), Some(2));
        assert_eq!(client_browser_syscall(&ui(86, &[]), &mut memory, &mut browser).unwrap(), None);
    }

    #[test]
    fn legacy_extension_servers() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut browser = FakeBrowser { log: Vec::new() };
        let count = HostCall::extension(QvmRole::Ui, 46, &[], AbiProfile::Legacy);
        assert_eq!(client_browser_syscall(&count, &mut memory, &mut browser).unwrap(), Some(10));
        let count2 = HostCall::extension(QvmRole::Ui, 48, &[], AbiProfile::Legacy);
        assert_eq!(client_browser_syscall(&count2, &mut memory, &mut browser).unwrap(), Some(12));
        let addr = HostCall::extension(QvmRole::Ui, 47, &[0, 256, 64], AbiProfile::Legacy);
        assert_eq!(client_browser_syscall(&addr, &mut memory, &mut browser).unwrap(), Some(0));
        assert_eq!(memory.read_string(256).unwrap(), "5.6.7.8:27960");
        let modern = HostCall::extension(QvmRole::Ui, 46, &[], AbiProfile::Modern);
        assert_eq!(client_browser_syscall(&modern, &mut memory, &mut browser).unwrap(), None);
        let game = HostCall::engine(QvmRole::Cgame, 46, &[], AbiProfile::Modern);
        assert_eq!(client_browser_syscall(&game, &mut memory, &mut browser).unwrap(), None);
    }
}
