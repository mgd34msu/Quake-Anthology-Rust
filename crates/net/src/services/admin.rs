//! Server administration ported from `src/network/services/admin.ts`.
//!
//! Quake II `g_svcmds.c` address masks and Quake III `sv_main.c` rcon. The
//! donor serializes rcon through a promise tail; this port answers
//! synchronously through a blocking host.

use std::collections::HashMap;

use thiserror::Error;

use crate::common::endpoint::{address_key, NetworkAddress};

/// IPv4 filter (`Ipv4Filter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Filter {
    /// Mask in little-endian host order.
    pub mask: u32,
    /// Comparison value in little-endian host order.
    pub compare: u32,
}

/// Source `addip` filter: omitted/zero octets are wildcards and supplied
/// octets truncate to bytes (`sourceIpv4Filter`).
#[must_use]
pub fn source_ipv4_filter(text: &str) -> Option<Ipv4Filter> {
    let mut bytes = [0u8; 4];
    let mut mask = [0u8; 4];
    let chars: Vec<char> = text.chars().collect();
    let mut cursor = 0;
    for index in 0..4 {
        let start = cursor;
        while cursor < chars.len() && chars[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if start == cursor {
            return None;
        }
        let value: u32 = text[start..cursor].parse().unwrap_or(0);
        let truncated = (value & 255) as u8;
        bytes[index] = truncated;
        if truncated != 0 {
            mask[index] = 255;
        }
        if cursor >= chars.len() {
            break;
        }
        cursor += 1;
    }
    Some(Ipv4Filter {
        mask: u32::from_le_bytes(mask),
        compare: u32::from_le_bytes(bytes),
    })
}

/// CIDR filter (`cidrIpv4Filter`).
pub fn cidr_ipv4_filter(address: &[u8; 4], prefix_bits: u32) -> Result<Ipv4Filter, AdminError> {
    if prefix_bits > 32 {
        return Err(AdminError::BadPrefix);
    }
    let mut mask = [0u8; 4];
    for (index, slot) in mask.iter_mut().enumerate() {
        let index = index as u32;
        *slot = if prefix_bits >= index * 8 + 8 {
            255
        } else if prefix_bits <= index * 8 {
            0
        } else {
            (255u32 << (8 - (prefix_bits - index * 8))) as u8 & 255
        };
    }
    let mask_value = u32::from_le_bytes(mask);
    let value = u32::from_le_bytes(*address);
    Ok(Ipv4Filter {
        mask: mask_value,
        compare: (value & mask_value),
    })
}

/// Error for administration failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AdminError {
    /// Invalid IPv4 prefix.
    #[error("Invalid IPv4 prefix")]
    BadPrefix,
    /// Invalid flood limit.
    #[error("Invalid flood limit")]
    BadFloodLimit,
    /// Invalid source chat flood policy.
    #[error("Invalid source chat flood policy")]
    BadChatPolicy,
    /// Rcon output buffer must hold a complete character and terminator.
    #[error("Rcon output buffer must hold a complete character and terminator")]
    BadOutput,
    /// Source rcon output requires byte characters.
    #[error("Source rcon output requires byte characters")]
    WideOutput,
}

/// Filter list mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterMode {
    /// Reject matches.
    #[default]
    DenyMatches,
    /// Reject non-matches.
    AllowMatches,
}

/// IPv4 filter list (`IpFilterList`).
#[derive(Debug, Default)]
pub struct IpFilterList {
    entries: Vec<Ipv4Filter>,
    /// List mode.
    pub mode: FilterMode,
}

impl IpFilterList {
    /// Create a list with a mode.
    #[must_use]
    pub fn new(mode: FilterMode) -> Self {
        Self {
            entries: Vec::new(),
            mode,
        }
    }

    /// Add a filter.
    pub fn add(&mut self, filter: Ipv4Filter) {
        self.entries.push(filter);
    }

    /// Remove a filter.
    pub fn remove(&mut self, filter: &Ipv4Filter) -> bool {
        if let Some(index) = self
            .entries
            .iter()
            .position(|value| value.mask == filter.mask && value.compare == filter.compare)
        {
            self.entries.remove(index);
            true
        } else {
            false
        }
    }

    /// True when the address is rejected (`rejects`).
    #[must_use]
    pub fn rejects(&self, address: &NetworkAddress) -> bool {
        let NetworkAddress::Ipv4 { host, .. } = address else {
            return false;
        };
        let value = u32::from_le_bytes(*host);
        let matches = self
            .entries
            .iter()
            .any(|filter| (value & filter.mask) == filter.compare);
        match self.mode {
            FilterMode::DenyMatches => matches,
            FilterMode::AllowMatches => !matches,
        }
    }

    /// Snapshot entries.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Ipv4Filter> {
        self.entries.clone()
    }
}

/// Token-bucket flood limiter (`FloodLimiter`).
#[derive(Debug)]
pub struct FloodLimiter {
    entries: HashMap<String, FloodEntry>,
    /// Burst tokens.
    pub burst: f64,
    /// Milliseconds per token.
    pub interval_milliseconds: f64,
}

#[derive(Debug, Clone)]
struct FloodEntry {
    tokens: f64,
    time: f64,
}

impl FloodLimiter {
    /// Create a limiter.
    pub fn new(burst: f64, interval_milliseconds: f64) -> Result<Self, AdminError> {
        if burst <= 0.0 || interval_milliseconds <= 0.0 {
            return Err(AdminError::BadFloodLimit);
        }
        Ok(Self {
            entries: HashMap::new(),
            burst,
            interval_milliseconds,
        })
    }

    /// Consume a token for an address (`allow`).
    pub fn allow(&mut self, address: &NetworkAddress, now: f64) -> bool {
        let key = address_key(address, false);
        let tokens = match self.entries.get(&key) {
            None => self.burst,
            Some(old) => (self.burst).min(old.tokens + 0f64.max(now - old.time) / self.interval_milliseconds),
        };
        self.entries.insert(
            key,
            FloodEntry {
                tokens: if tokens >= 1.0 { tokens - 1.0 } else { tokens },
                time: now,
            },
        );
        tokens >= 1.0
    }

    /// Expire idle entries (`expire`).
    pub fn expire(&mut self, now: f64) {
        self.entries
            .retain(|_, entry| now - entry.time <= self.burst * self.interval_milliseconds);
    }
}

/// Per-client source chat flood ring (`SourceChatFlood`).
#[derive(Debug)]
pub struct SourceChatFlood {
    times: [f64; 10],
    head: usize,
    locked_until: f64,
    messages: usize,
    seconds: f64,
    lock_seconds: f64,
}

/// Chat flood verdict.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatVerdict {
    /// Allowed.
    Allowed,
    /// Locked with remaining seconds.
    Locked {
        /// Remaining seconds.
        seconds: i64,
    },
    /// Flooding with lock seconds.
    Flood {
        /// Lock seconds.
        seconds: i64,
    },
}

impl SourceChatFlood {
    /// Create a chat flood policy.
    pub fn new(messages: usize, seconds: f64, lock_seconds: f64) -> Result<Self, AdminError> {
        if messages > 10 || seconds < 0.0 || lock_seconds < 0.0 {
            return Err(AdminError::BadChatPolicy);
        }
        Ok(Self {
            times: [0.0; 10],
            head: 0,
            locked_until: 0.0,
            messages,
            seconds,
            lock_seconds,
        })
    }

    /// Check a chat message (`check`).
    pub fn check(&mut self, now: f64, paused: bool) -> ChatVerdict {
        if self.messages == 0 {
            return ChatVerdict::Allowed;
        }
        if !paused && now < self.locked_until {
            return ChatVerdict::Locked {
                seconds: (self.locked_until - now).trunc() as i64,
            };
        }
        let previous = self.times[(self.head + 11 - self.messages) % 10];
        if !paused && previous != 0.0 && now - previous < self.seconds {
            self.locked_until = now + self.lock_seconds;
            return ChatVerdict::Flood {
                seconds: self.lock_seconds.trunc() as i64,
            };
        }
        self.head = (self.head + 1) % 10;
        self.times[self.head] = now;
        ChatVerdict::Allowed
    }
}

/// Q3 `SVC_RemoteCommand` command extraction: advances past the password
/// without retokenizing command quotes (`q3RconCommand`).
#[must_use]
pub fn q3_rcon_command(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut cursor = 4;
    while chars.get(cursor) == Some(&' ') {
        cursor += 1;
    }
    while cursor < chars.len() && chars[cursor] != ' ' {
        cursor += 1;
    }
    while chars.get(cursor) == Some(&' ') {
        cursor += 1;
    }
    chars[cursor..].iter().take(1023).collect()
}

/// Rcon result (`RconResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RconResult {
    /// Throttled (500ms between commands).
    Throttled,
    /// No password set.
    Disabled,
    /// Bad password.
    Denied,
    /// Executed.
    Executed,
}

/// Rcon profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RconProfile {
    /// Quake III byte output.
    #[default]
    Q3,
    /// Unified UTF-8 output.
    Unified,
}

/// Rcon host (`RconHost`).
pub trait RconHost {
    /// Current password.
    fn password(&self) -> String;
    /// Execute a command, streaming output.
    fn execute(&mut self, command: &str, output: &mut dyn FnMut(&str)) -> Result<(), AdminError>;
    /// Reply to an address.
    fn reply(&mut self, to: &NetworkAddress, text: &str);
}

/// Rcon service (`RconService`).
pub struct RconService<'a> {
    host: &'a mut dyn RconHost,
    profile: RconProfile,
    output_bytes: usize,
    last_time: f64,
}

impl<'a> RconService<'a> {
    /// Create a service over a host.
    pub fn new(
        host: &'a mut dyn RconHost,
        profile: RconProfile,
        output_bytes: usize,
    ) -> Result<Self, AdminError> {
        if output_bytes < 5 {
            return Err(AdminError::BadOutput);
        }
        Ok(Self {
            host,
            profile,
            output_bytes,
            last_time: 0.0,
        })
    }

    /// Handle an rcon request (`handle`).
    pub fn handle(
        &mut self,
        from: &NetworkAddress,
        supplied_password: &str,
        command: &str,
        milliseconds: f64,
    ) -> Result<RconResult, AdminError> {
        let time = if self.profile == RconProfile::Q3 {
            (milliseconds as i64 as u32) as f64
        } else {
            milliseconds
        };
        let next = if self.profile == RconProfile::Q3 {
            ((self.last_time as i64 as u32).wrapping_add(500)) as f64
        } else {
            self.last_time + 500.0
        };
        if time < next {
            return Ok(RconResult::Throttled);
        }
        self.last_time = time;
        let password = self.host.password();
        if password.is_empty() {
            self.host.reply(from, "No rconpassword set on the server.\n");
            return Ok(RconResult::Disabled);
        }
        if password != supplied_password {
            self.host.reply(from, "Bad rconpassword.\n");
            return Ok(RconResult::Denied);
        }
        let profile = self.profile;
        let output_bytes = self.output_bytes;
        let mut chunks: Vec<String> = Vec::new();
        let mut buffered = String::new();
        let mut failed = None;
        if !command.is_empty() {
            let command = if profile == RconProfile::Q3 {
                command.chars().take(1023).collect::<String>()
            } else {
                command.to_owned()
            };
            let host = &mut self.host;
            let executed = host.execute(&command, &mut |text| {
                if failed.is_some() {
                    return;
                }
                for character in text.chars() {
                    if profile == RconProfile::Q3 && (character as u32) > 255 {
                        failed = Some(AdminError::WideOutput);
                        return;
                    }
                    buffered.push(character);
                    let size = if profile == RconProfile::Q3 {
                        buffered.chars().count()
                    } else {
                        buffered.len()
                    };
                    if size > output_bytes - 1 {
                        chunks.push(std::mem::take(&mut buffered));
                    }
                }
            });
            if let Err(error) = executed {
                failed = Some(error);
            }
        }
        if !buffered.is_empty() {
            chunks.push(std::mem::take(&mut buffered));
        }
        for chunk in &chunks {
            self.host.reply(from, chunk);
        }
        if let Some(error) = failed {
            return Err(error);
        }
        Ok(RconResult::Executed)
    }
}

/// Server administration surface (`ServerAdministration`).
pub trait ServerAdministration {
    /// Rcon password.
    fn rcon_password(&self) -> String;
    /// Execute a command, streaming output.
    fn execute(&mut self, command: &str, output: &mut dyn FnMut(&str));
    /// True when the address is rejected.
    fn rejects(&self, address: &NetworkAddress) -> bool;
    /// Master servers.
    fn masters(&self) -> Vec<NetworkAddress>;
    /// Record an rcon event.
    fn record(&mut self, address: &NetworkAddress, result: RconResult);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::endpoint::ipv4_address;

    #[test]
    fn source_filter_masks_zero_octets() {
        let filter = source_ipv4_filter("192.168.0.0").unwrap();
        let address = ipv4_address([192, 168, 1, 7], 27910, false).unwrap();
        let mut list = IpFilterList::new(FilterMode::DenyMatches);
        list.add(filter);
        assert!(list.rejects(&address));
        let other = ipv4_address([10, 0, 0, 1], 27910, false).unwrap();
        assert!(!list.rejects(&other));
        assert!(source_ipv4_filter("nope").is_none());
    }

    #[test]
    fn rcon_command_skips_password() {
        assert_eq!(q3_rcon_command("rcon secret say \"hi there\""), "say \"hi there\"");
    }

    #[test]
    fn chat_flood_locks_repeat_offenders() {
        let mut flood = SourceChatFlood::new(2, 10.0, 30.0).unwrap();
        assert_eq!(flood.check(100.0, false), ChatVerdict::Allowed);
        assert_eq!(flood.check(101.0, false), ChatVerdict::Allowed);
        assert!(matches!(flood.check(102.0, false), ChatVerdict::Flood { .. }));
        assert!(matches!(flood.check(103.0, false), ChatVerdict::Locked { .. }));
    }
}
