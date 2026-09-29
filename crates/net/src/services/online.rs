//! Online accounts, lobbies, and rankings ported from
//! `src/network/services/online.ts`.
//!
//! Usable server-owned accounts; retail authorization is a separate selected
//! provider. Passwords hash with the crate's salted SHA-256 verifier (see
//! [`crate::common::hash`]) instead of donor scrypt, and randomness reads
//! from the operating system (`/dev/urandom` with a time-seeded fallback).

use std::collections::{BTreeMap, HashMap};
use std::io::Read;

use qa_core::identity::ProviderId;
use thiserror::Error;

use crate::common::endpoint::NetworkAddress;
use crate::common::hash::{hex_lower, password_verifier, timing_safe_equal, Sha256};
use crate::common::session::{canonical, CompositionIdentity, Json, WireSelection};
use super::json::{parse_json, JsonError};

/// Error for online service failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OnlineError {
    /// Account requires an unused name and a password.
    #[error("Account requires an unused name and a password")]
    BadAccount,
    /// Invalid account credentials.
    #[error("Invalid account credentials")]
    BadCredentials,
    /// Invalid local account save.
    #[error("Invalid local account save")]
    BadAccountSave,
    /// Invalid account record.
    #[error("Invalid account record")]
    BadAccountRecord,
    /// Invalid account fields.
    #[error("Invalid account fields")]
    BadAccountFields,
    /// Invalid lobby settings.
    #[error("Invalid lobby settings")]
    BadLobby,
    /// Lobby no longer exists.
    #[error("Lobby no longer exists")]
    MissingLobby,
    /// Cannot join this lobby.
    #[error("Cannot join this lobby")]
    CannotJoin,
    /// Lobby has insufficient seat capacity.
    #[error("Lobby has insufficient seat capacity")]
    LobbyFull,
    /// Account cannot change readiness.
    #[error("Account cannot change readiness")]
    CannotReady,
    /// Lobby is not ready to start.
    #[error("Lobby is not ready to start")]
    NotReady,
    /// Lobby launch is no longer current.
    #[error("Lobby launch is no longer current")]
    StaleLaunch,
    /// Bound lobby wire does not match its prepared composition.
    #[error("Bound lobby wire does not match its prepared composition")]
    WireMismatch,
    /// Only the lobby owner can complete its match.
    #[error("Only the lobby owner can complete its match")]
    NotOwner,
    /// Invalid ranking report.
    #[error("Invalid ranking report")]
    BadReport,
    /// Invalid ranked player.
    #[error("Invalid ranked player")]
    BadPlayer,
    /// Invalid rank statistic.
    #[error("Invalid rank statistic")]
    BadStatistic,
    /// Invalid ranking save.
    #[error("Invalid ranking save")]
    BadRankingSave,
    /// Duplicate saved match.
    #[error("Duplicate saved match")]
    DuplicateMatch,
    /// Save JSON is malformed.
    #[error("Invalid save JSON: {0}")]
    BadJson(#[from] JsonError),
}

/// Account id (`account:{32 hex}`).
pub type AccountId = String;
/// Lobby id (`lobby:{32 hex}`).
pub type LobbyId = String;
/// Access token (`access:{64 hex}`).
pub type AccessToken = String;

/// Account (`Account`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Account {
    /// Account id.
    pub id: AccountId,
    /// Account name.
    pub name: String,
}

/// Authorization result (`Authorization`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authorization {
    /// Authorized with an access token.
    Authorized {
        /// Account.
        account: Account,
        /// Access token.
        token: AccessToken,
    },
    /// Denied.
    Denied {
        /// Reason.
        reason: String,
    },
    /// Provider unavailable.
    Unavailable {
        /// Provider.
        provider: String,
        /// Reason.
        reason: String,
    },
}

/// Authorization provider (`AuthorizationProvider`).
pub trait AuthorizationProvider {
    /// Authorize a name/credential pair.
    fn authorize(&mut self, name: &str, credential: &str) -> Authorization;
}

/// Password hashing iterations for [`password_verifier`].
pub const PASSWORD_ITERATIONS: u32 = 4096;

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    let mut filled = false;
    if let Ok(mut urandom) = std::fs::File::open("/dev/urandom") {
        if urandom.read_exact(&mut buffer).is_ok() {
            filled = true;
        }
    }
    if !filled {
        // Time-seeded fallback when no OS randomness is available.
        let seed = format!(
            "{:?}-{}-{}",
            std::time::SystemTime::now(),
            std::process::id(),
            buffer.as_ptr() as usize
        );
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        let digest = hasher.finish();
        for (index, slot) in buffer.iter_mut().enumerate() {
            *slot = digest[index % digest.len()];
        }
    }
    hex_lower(&buffer)
}

struct AccountRecord {
    account: Account,
    salt: String,
    password_hash: String,
}

/// Server-owned local accounts (`LocalAuthorization`).
#[derive(Default)]
pub struct LocalAuthorization {
    accounts: HashMap<String, AccountRecord>,
    tokens: HashMap<AccessToken, Account>,
}

impl LocalAuthorization {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an account (`register`).
    pub fn register(&mut self, name: &str, password: &str) -> Result<Account, OnlineError> {
        if name.trim().is_empty() || password.is_empty() || self.accounts.contains_key(name) {
            return Err(OnlineError::BadAccount);
        }
        let account = Account {
            id: format!("account:{}", random_hex(16)),
            name: name.to_owned(),
        };
        let salt = random_hex(16);
        let password_hash = password_verifier(password, &salt, PASSWORD_ITERATIONS);
        self.accounts.insert(name.to_owned(), AccountRecord {
            account: account.clone(),
            salt,
            password_hash,
        });
        Ok(account)
    }

    /// Look up the account for a token (`account`).
    #[must_use]
    pub fn account(&self, token: &str) -> Option<&Account> {
        self.tokens.get(token)
    }

    /// Revoke a token (`revoke`).
    pub fn revoke(&mut self, token: &str) {
        self.tokens.remove(token);
    }

    /// Serialize accounts (`save`).
    #[must_use]
    pub fn save(&self) -> String {
        let mut accounts: Vec<Json> = self
            .accounts
            .values()
            .map(|record| {
                let mut account = BTreeMap::new();
                account.insert("id".to_owned(), Json::String(record.account.id.clone()));
                account.insert("name".to_owned(), Json::String(record.account.name.clone()));
                let mut row = BTreeMap::new();
                row.insert("account".to_owned(), Json::Object(account));
                row.insert("salt".to_owned(), Json::String(record.salt.clone()));
                row.insert("passwordHash".to_owned(), Json::String(record.password_hash.clone()));
                Json::Object(row)
            })
            .collect();
        accounts.sort_by(|a, b| canonical(a).unwrap_or_default().cmp(&canonical(b).unwrap_or_default()));
        let mut root = BTreeMap::new();
        root.insert("version".to_owned(), Json::Number(1.0));
        root.insert("accounts".to_owned(), Json::Array(accounts));
        canonical(&Json::Object(root)).unwrap_or_else(|_| String::new())
    }

    /// Restore accounts, clearing tokens (`restore`).
    pub fn restore(&mut self, text: &str) -> Result<(), OnlineError> {
        let value = parse_json(text)?;
        let Json::Object(root) = &value else {
            return Err(OnlineError::BadAccountSave);
        };
        if root.get("version") != Some(&Json::Number(1.0)) {
            return Err(OnlineError::BadAccountSave);
        }
        let Some(Json::Array(rows)) = root.get("accounts") else {
            return Err(OnlineError::BadAccountSave);
        };
        let mut restored: HashMap<String, AccountRecord> = HashMap::new();
        let mut ids = std::collections::HashSet::new();
        for row in rows {
            let Json::Object(row) = row else {
                return Err(OnlineError::BadAccountRecord);
            };
            let Some(Json::Object(account)) = row.get("account") else {
                return Err(OnlineError::BadAccountRecord);
            };
            let (Some(Json::String(id)), Some(Json::String(name)), Some(Json::String(salt)), Some(Json::String(password_hash))) =
                (account.get("id"), account.get("name"), row.get("salt"), row.get("passwordHash"))
            else {
                return Err(OnlineError::BadAccountFields);
            };
            if !is_account_id(id)
                || name.is_empty()
                || !is_hex(salt, 32)
                || !is_hex(password_hash, 64)
                || restored.contains_key(name)
                || !ids.insert(id.clone())
            {
                return Err(OnlineError::BadAccountFields);
            }
            restored.insert(name.clone(), AccountRecord {
                account: Account {
                    id: id.clone(),
                    name: name.clone(),
                },
                salt: salt.clone(),
                password_hash: password_hash.clone(),
            });
        }
        self.accounts = restored;
        self.tokens.clear();
        Ok(())
    }
}

fn is_hex(text: &str, length: usize) -> bool {
    text.len() == length && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_account_id(value: &str) -> bool {
    value.len() == 40 && value.starts_with("account:") && is_hex(&value[8..], 32)
}

impl AuthorizationProvider for LocalAuthorization {
    fn authorize(&mut self, name: &str, credential: &str) -> Authorization {
        let Some(record) = self.accounts.get(name) else {
            return Authorization::Denied {
                reason: "Invalid account credentials".to_owned(),
            };
        };
        let supplied = password_verifier(credential, &record.salt, PASSWORD_ITERATIONS);
        if !timing_safe_equal(supplied.as_bytes(), record.password_hash.as_bytes()) {
            return Authorization::Denied {
                reason: "Invalid account credentials".to_owned(),
            };
        }
        let token = format!("access:{}", random_hex(32));
        self.tokens.insert(token.clone(), record.account.clone());
        Authorization::Authorized {
            account: record.account.clone(),
            token,
        }
    }
}

/// Lobby member (`LobbyMember`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyMember {
    /// Member account.
    pub account: Account,
    /// Seats held.
    pub seats: u32,
    /// Ready flag.
    pub ready: bool,
}

/// Lobby phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyPhase {
    /// Open for joins.
    Open,
    /// Starting.
    Starting,
    /// Playing with a bound endpoint and wire.
    Playing {
        /// Server endpoint.
        endpoint: NetworkAddress,
        /// Bound wire.
        wire: WireSelection,
    },
}

/// Lobby (`Lobby`).
#[derive(Debug, Clone, PartialEq)]
pub struct Lobby {
    /// Lobby id.
    pub id: LobbyId,
    /// Owner account id.
    pub owner: AccountId,
    /// Lobby name.
    pub name: String,
    /// Seat capacity.
    pub capacity: u32,
    /// Prepared composition.
    pub composition: CompositionIdentity,
    /// Members.
    pub members: Vec<LobbyMember>,
    /// Match generation.
    pub match_generation: u64,
    /// Phase.
    pub phase: LobbyPhase,
}

/// Local lobby service (`LocalLobbyService`).
#[derive(Default)]
pub struct LocalLobbyService {
    lobbies: HashMap<LobbyId, Lobby>,
}

impl LocalLobbyService {
    /// Create an empty service.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a lobby (`create`).
    pub fn create(
        &mut self,
        owner: &Account,
        name: &str,
        capacity: u32,
        composition: CompositionIdentity,
        seats: u32,
    ) -> Result<Lobby, OnlineError> {
        if capacity < 1 || seats < 1 || seats > capacity || name.trim().is_empty() {
            return Err(OnlineError::BadLobby);
        }
        let lobby = Lobby {
            id: format!("lobby:{}", random_hex(16)),
            owner: owner.id.clone(),
            name: name.to_owned(),
            capacity,
            composition,
            members: vec![LobbyMember {
                account: owner.clone(),
                seats,
                ready: false,
            }],
            phase: LobbyPhase::Open,
            match_generation: 0,
        };
        self.lobbies.insert(lobby.id.clone(), lobby.clone());
        Ok(lobby)
    }

    fn require(&self, id: &str) -> Result<Lobby, OnlineError> {
        self.lobbies.get(id).cloned().ok_or(OnlineError::MissingLobby)
    }

    /// List lobbies (`list`).
    #[must_use]
    pub fn list(&self) -> Vec<Lobby> {
        self.lobbies.values().cloned().collect()
    }

    /// Join a lobby (`join`).
    pub fn join(&mut self, id: &str, account: &Account, seats: u32) -> Result<Lobby, OnlineError> {
        let lobby = self.require(id)?;
        if lobby.phase != LobbyPhase::Open
            || lobby.members.iter().any(|member| member.account.id == account.id)
        {
            return Err(OnlineError::CannotJoin);
        }
        if seats < 1
            || lobby.members.iter().map(|member| member.seats).sum::<u32>() + seats > lobby.capacity
        {
            return Err(OnlineError::LobbyFull);
        }
        let mut updated = lobby;
        updated.members.push(LobbyMember {
            account: account.clone(),
            seats,
            ready: false,
        });
        self.lobbies.insert(id.to_owned(), updated.clone());
        Ok(updated)
    }

    /// Change readiness (`ready`).
    pub fn ready(&mut self, id: &str, account: &AccountId, ready: bool) -> Result<Lobby, OnlineError> {
        let lobby = self.require(id)?;
        if lobby.phase != LobbyPhase::Open
            || !lobby.members.iter().any(|member| &member.account.id == account)
        {
            return Err(OnlineError::CannotReady);
        }
        let mut updated = lobby;
        for member in &mut updated.members {
            if &member.account.id == account {
                member.ready = ready;
            }
        }
        self.lobbies.insert(id.to_owned(), updated.clone());
        Ok(updated)
    }

    /// Start a match (`start`).
    pub fn start(&mut self, id: &str, owner: &AccountId) -> Result<Lobby, OnlineError> {
        let lobby = self.require(id)?;
        if &lobby.owner != owner || lobby.phase != LobbyPhase::Open || lobby.members.iter().any(|member| !member.ready) {
            return Err(OnlineError::NotReady);
        }
        let mut updated = lobby;
        updated.phase = LobbyPhase::Starting;
        updated.match_generation += 1;
        self.lobbies.insert(id.to_owned(), updated.clone());
        Ok(updated)
    }

    /// Publish a bound endpoint and wire (`publish`).
    pub fn publish(
        &mut self,
        id: &str,
        owner: &AccountId,
        match_generation: u64,
        endpoint: NetworkAddress,
        wire: WireSelection,
    ) -> Result<Lobby, OnlineError> {
        let lobby = self.require(id)?;
        if &lobby.owner != owner || lobby.phase != LobbyPhase::Starting || lobby.match_generation != match_generation {
            return Err(OnlineError::StaleLaunch);
        }
        if let WireSelection::Unified {
            composition,
            snapshot_schema,
            ..
        } = &wire
        {
            if composition != &lobby.composition.digest
                || snapshot_schema != &lobby.composition.composition.snapshot_schema
            {
                return Err(OnlineError::WireMismatch);
            }
        }
        let mut updated = lobby;
        updated.phase = LobbyPhase::Playing { endpoint, wire };
        self.lobbies.insert(id.to_owned(), updated.clone());
        Ok(updated)
    }

    /// Complete a match generation (`complete`).
    pub fn complete(
        &mut self,
        id: &str,
        owner: &AccountId,
        match_generation: u64,
    ) -> Result<Lobby, OnlineError> {
        let lobby = self.require(id)?;
        if &lobby.owner != owner {
            return Err(OnlineError::NotOwner);
        }
        if lobby.phase == LobbyPhase::Open || lobby.match_generation != match_generation {
            return Ok(lobby);
        }
        let mut updated = lobby;
        updated.phase = LobbyPhase::Open;
        for member in &mut updated.members {
            member.ready = false;
        }
        self.lobbies.insert(id.to_owned(), updated.clone());
        Ok(updated)
    }

    /// Leave a lobby; the owner disbands it (`leave`).
    pub fn leave(&mut self, id: &str, account: &AccountId) -> Result<(), OnlineError> {
        let lobby = self.require(id)?;
        if &lobby.owner == account {
            self.lobbies.remove(id);
            return Ok(());
        }
        let mut updated = lobby;
        updated.members.retain(|member| &member.account.id != account);
        self.lobbies.insert(id.to_owned(), updated);
        Ok(())
    }
}

/// Ranked player report row.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedPlayer {
    /// Account id.
    pub account: AccountId,
    /// Score.
    pub score: f64,
    /// Won the match.
    pub won: bool,
    /// Statistics.
    pub statistics: BTreeMap<String, f64>,
}

/// Ranking report (`RankingReport`).
#[derive(Debug, Clone, PartialEq)]
pub struct RankingReport {
    /// Match name.
    pub match_name: String,
    /// Rules provider.
    pub rules: ProviderId,
    /// Players.
    pub players: Vec<RankedPlayer>,
}

/// Ranking entry (`RankingEntry`).
#[derive(Debug, Clone, PartialEq)]
pub struct RankingEntry {
    /// Account id.
    pub account: AccountId,
    /// Matches played.
    pub matches: u64,
    /// Wins.
    pub wins: u64,
    /// Total score.
    pub score: f64,
    /// Totals.
    pub statistics: BTreeMap<String, f64>,
}

/// Local ranking service (`LocalRankingService`).
#[derive(Default)]
pub struct LocalRankingService {
    reports: HashMap<String, RankingReport>,
}

impl LocalRankingService {
    /// Create an empty service.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Submit a report; `false` reports a duplicate (`submit`).
    pub fn submit(&mut self, report: RankingReport) -> Result<bool, OnlineError> {
        let key = format!("{}:{}/{}", report.rules.namespace, report.rules.name, report.match_name);
        if self.reports.contains_key(&key) {
            return Ok(false);
        }
        if report.match_name.is_empty()
            || report.players.iter().map(|player| &player.account).collect::<std::collections::HashSet<_>>().len()
                != report.players.len()
            || report.players.iter().any(|player| {
                !player.score.is_finite() || player.statistics.values().any(|value| !value.is_finite())
            })
        {
            return Err(OnlineError::BadReport);
        }
        self.reports.insert(key, report);
        Ok(true)
    }

    /// Standings for a rules provider (`standings`).
    #[must_use]
    pub fn standings(&self, rules: &ProviderId) -> Vec<RankingEntry> {
        let mut entries: HashMap<AccountId, RankingEntry> = HashMap::new();
        for report in self.reports.values() {
            if &report.rules != rules {
                continue;
            }
            for player in &report.players {
                let entry = entries.entry(player.account.clone()).or_insert_with(|| RankingEntry {
                    account: player.account.clone(),
                    matches: 0,
                    wins: 0,
                    score: 0.0,
                    statistics: BTreeMap::new(),
                });
                entry.matches += 1;
                entry.wins += u64::from(player.won);
                entry.score += player.score;
                for (key, value) in &player.statistics {
                    *entry.statistics.entry(key.clone()).or_insert(0.0) += value;
                }
            }
        }
        let mut standings: Vec<RankingEntry> = entries.into_values().collect();
        standings.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(right.wins.cmp(&left.wins))
                .then(left.account.cmp(&right.account))
        });
        standings
    }

    /// Serialize reports (`save`).
    #[must_use]
    pub fn save(&self) -> String {
        let mut rows: Vec<Json> = self
            .reports
            .values()
            .map(|report| {
                let mut row = BTreeMap::new();
                row.insert("match".to_owned(), Json::String(report.match_name.clone()));
                row.insert(
                    "rules".to_owned(),
                    Json::String(format!("{}:{}", report.rules.namespace, report.rules.name)),
                );
                row.insert(
                    "players".to_owned(),
                    Json::Array(
                        report
                            .players
                            .iter()
                            .map(|player| {
                                let mut row = BTreeMap::new();
                                row.insert("account".to_owned(), Json::String(player.account.clone()));
                                row.insert("score".to_owned(), Json::Number(player.score));
                                row.insert("won".to_owned(), Json::Bool(player.won));
                                row.insert(
                                    "statistics".to_owned(),
                                    Json::Array(
                                        player
                                            .statistics
                                            .iter()
                                            .map(|(key, value)| {
                                                Json::Array(vec![
                                                    Json::String(key.clone()),
                                                    Json::Number(*value),
                                                ])
                                            })
                                            .collect(),
                                    ),
                                );
                                Json::Object(row)
                            })
                            .collect(),
                    ),
                );
                Json::Object(row)
            })
            .collect();
        rows.sort_by(|a, b| canonical(a).unwrap_or_default().cmp(&canonical(b).unwrap_or_default()));
        canonical(&Json::Array(rows)).unwrap_or_else(|_| String::new())
    }

    /// Restore reports (`restore`).
    pub fn restore(&mut self, text: &str) -> Result<(), OnlineError> {
        let value = parse_json(text)?;
        let Json::Array(rows) = &value else {
            return Err(OnlineError::BadRankingSave);
        };
        let mut restored = LocalRankingService::new();
        for row in rows {
            let Json::Object(row) = row else {
                return Err(OnlineError::BadReport);
            };
            let (Some(Json::String(match_name)), Some(Json::String(rules)), Some(Json::Array(source_players))) =
                (row.get("match"), row.get("rules"), row.get("players"))
            else {
                return Err(OnlineError::BadReport);
            };
            let Some((namespace, name)) = rules.split_once(':') else {
                return Err(OnlineError::BadReport);
            };
            let mut players = Vec::new();
            for player in source_players {
                let Json::Object(player) = player else {
                    return Err(OnlineError::BadPlayer);
                };
                let (Some(Json::String(account)), Some(Json::Number(score)), Some(Json::Bool(won)), Some(Json::Array(pairs))) =
                    (player.get("account"), player.get("score"), player.get("won"), player.get("statistics"))
                else {
                    return Err(OnlineError::BadPlayer);
                };
                if !is_account_id(account) {
                    return Err(OnlineError::BadPlayer);
                }
                let mut statistics = BTreeMap::new();
                for pair in pairs {
                    let Json::Array(pair) = pair else {
                        return Err(OnlineError::BadStatistic);
                    };
                    if pair.len() != 2 {
                        return Err(OnlineError::BadStatistic);
                    }
                    let (Some(Json::String(key)), Some(Json::Number(value))) =
                        (pair.first(), pair.get(1))
                    else {
                        return Err(OnlineError::BadStatistic);
                    };
                    statistics.insert(key.clone(), *value);
                }
                players.push(RankedPlayer {
                    account: account.clone(),
                    score: *score,
                    won: *won,
                    statistics,
                });
            }
            if !restored.submit(RankingReport {
                match_name: match_name.clone(),
                rules: ProviderId::new(namespace, name),
                players,
            })? {
                return Err(OnlineError::DuplicateMatch);
            }
        }
        self.reports = restored.reports;
        Ok(())
    }
}

/// Add-on catalog entry (`AddonCatalogEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddonCatalogEntry {
    /// Content id.
    pub content: String,
    /// Title.
    pub title: String,
    /// Installed flag.
    pub installed: bool,
    /// Download URL, if any.
    pub download: Option<String>,
}

/// Add-on catalog provider (`AddonCatalogProvider`).
pub trait AddonCatalogProvider {
    /// List catalog entries.
    fn list(&mut self) -> Vec<AddonCatalogEntry>;
}

/// External service state (`ExternalServiceState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalServiceState {
    /// Available.
    Available {
        /// Provider.
        provider: String,
    },
    /// Unavailable.
    Unavailable {
        /// Provider.
        provider: String,
        /// Reason.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::session::{composition_identity, SessionComposition};

    fn composition() -> CompositionIdentity {
        composition_identity(&SessionComposition {
            recipe: Json::Object(BTreeMap::new()),
            snapshot_schema: ProviderId::new("test", "snap"),
            actor_configurations: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn accounts_authorize_and_persist() {
        let mut registry = LocalAuthorization::new();
        let account = registry.register("player", "secret").unwrap();
        assert!(registry.register("player", "other").is_err());
        let authorized = registry.authorize("player", "secret");
        let token = match authorized {
            Authorization::Authorized { token, .. } => token,
            _ => panic!("expected authorization"),
        };
        assert_eq!(registry.account(&token).unwrap().id, account.id);
        assert!(matches!(
            registry.authorize("player", "wrong"),
            Authorization::Denied { .. }
        ));
        let saved = registry.save();
        let mut restored = LocalAuthorization::new();
        restored.restore(&saved).unwrap();
        assert!(matches!(
            restored.authorize("player", "secret"),
            Authorization::Authorized { .. }
        ));
    }

    #[test]
    fn lobbies_run_to_start() {
        let mut registry = LocalAuthorization::new();
        let owner = registry.register("owner", "pw").unwrap();
        let guest = registry.register("guest", "pw").unwrap();
        let mut lobbies = LocalLobbyService::new();
        let lobby = lobbies.create(&owner, "match", 4, composition(), 1).unwrap();
        lobbies.join(&lobby.id, &guest, 1).unwrap();
        lobbies.ready(&lobby.id, &owner.id, true).unwrap();
        lobbies.ready(&lobby.id, &guest.id, true).unwrap();
        let started = lobbies.start(&lobby.id, &owner.id).unwrap();
        assert_eq!(started.phase, LobbyPhase::Starting);
        assert_eq!(started.match_generation, 1);
    }

    #[test]
    fn rankings_accumulate_and_persist() {
        let mut service = LocalRankingService::new();
        assert!(service
            .submit(RankingReport {
                match_name: "m1".to_owned(),
                rules: ProviderId::new("q3", "dm"),
                players: vec![RankedPlayer {
                    account: format!("account:{}", "a".repeat(32)),
                    score: 10.0,
                    won: true,
                    statistics: BTreeMap::from([("frags".to_owned(), 10.0)]),
                }],
            })
            .unwrap());
        let standings = service.standings(&ProviderId::new("q3", "dm"));
        assert_eq!(standings.len(), 1);
        assert_eq!(standings[0].wins, 1);
        let saved = service.save();
        let mut restored = LocalRankingService::new();
        restored.restore(&saved).unwrap();
        assert_eq!(restored.standings(&ProviderId::new("q3", "dm")).len(), 1);
    }
}
