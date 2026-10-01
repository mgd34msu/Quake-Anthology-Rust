//! Quake III rankings matches over the shared lifecycle.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-rankings.ts`
//! (`Q3RankingHost`, `ApplicationQ3Rankings`). The lifecycle, entity pool, level, and
//! ranking tables are the ported [`RankingLifecycle`], [`EntityPool`], [`GameLevel`], and
//! table types; the host arrives through the [`Q3RankingHost`] seam. The donor's async
//! provider calls are sync because the merged lifecycle is sync, and kind changes compare
//! state discriminants exactly like the donor's `kind` checks. The report sink needs
//! shared ownership, so the report queue lives behind one `Rc<RefCell<..>>` while the
//! lifecycle (which borrows the provider) stays outside it; methods sync the sink's
//! accepting flag after every lifecycle state change, and a sink overflow surfaces as
//! `unavailable` at the next flush instead of synchronously. Frame scopes its borrows
//! because weapon-time reports re-enter through the pool.

use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use qa_client::ui::settings::rankings::{
    RankingAccount, RankingAccountActions, RankingAccountRequest as UiAccountRequest, RankingPlayerView,
    RankingServiceView,
};
use qa_content::q3::base::game::entities::EntityPool;
use qa_content::q3::base::game::level::GameLevel;
use qa_content::q3::base::game::rankings::Q3RankingReport;
use qa_content::q3::base::game::state::Q3GameError;
use qa_content::q3::base::shared::definitions::{GameType, Team};
use qa_content::q3::base::shared::entity_shared::ServerEntityFlags;
use qa_net::services::rankings::{
    RankingAccountRequest, RankingError, RankingLifecycle, RankingPlayerState, RankingServiceProvider,
    RankingServiceState,
};
use thiserror::Error;

/// Match host (donor `Q3RankingHost`).
///
/// Source slots are local to this match. Account IDs never become ActorIds or local
/// progress identities.
pub trait Q3RankingHost {
    /// Player state changed.
    fn status(&mut self, slot: i32, state: &RankingPlayerState);
    /// Service state changed.
    fn service_status(&mut self, state: &RankingServiceState) {
        let _ = state;
    }
    /// Show the rankings menu.
    fn menu(&mut self, slot: i32);
    /// Move a slot to spectator.
    fn spectator(&mut self, slot: i32);
    /// Activate a slot.
    fn activate(&mut self, slot: i32);
    /// Refresh a scoreboard.
    fn scoreboard(&mut self, slot: i32);
    /// Drop a bot.
    fn drop_bot(&mut self, slot: i32);
    /// Current game type number.
    fn game_type(&self) -> i32;
    /// Read a cvar.
    fn cvar(&self, name: &str) -> String;
    /// Write a cvar.
    fn set_cvar(&mut self, name: &str, value: &str);
}

/// Failure of a rankings operation, with donor messages.
#[derive(Debug, Error)]
pub enum Q3RankingsError {
    /// The match owner is closed.
    #[error("Ranking match owner is closed")]
    Closed,
    /// The slot has no connected source client.
    #[error("Ranking account requires a connected source client")]
    NoClient,
    /// Lifecycle failure.
    #[error(transparent)]
    Rank(#[from] RankingError),
    /// Table attach failure.
    #[error(transparent)]
    Attach(#[from] Q3GameError),
}

/// One watched weapon (donor `weapons` entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Q3WeaponWatch {
    weapon: i32,
    since: i32,
}

/// Shared mutable match state behind the report sink.
struct Shared {
    reports: Vec<Q3RankingReport>,
    observed: HashMap<i32, RankingPlayerState>,
    weapons: HashMap<i32, Q3WeaponWatch>,
    closed: bool,
    ended: bool,
    accepting: bool,
    overflowed: bool,
}

/// Parse a numeric cvar the way JavaScript `Number(...)` does for decimal text.
fn js_number(text: &str) -> f64 {
    let text = text.trim();
    if text.is_empty() {
        return 0.0;
    }
    text.parse::<f64>().unwrap_or(0.0)
}

/// Quake III rankings match (donor `ApplicationQ3Rankings`).
pub struct ApplicationQ3Rankings<'a, H: 'a> {
    pool: Rc<EntityPool>,
    level: Rc<RefCell<GameLevel>>,
    host: Rc<RefCell<H>>,
    lifecycle: Rc<RefCell<RankingLifecycle<'a>>>,
    shared: Rc<RefCell<Shared>>,
    detach: Rc<dyn Fn()>,
}

impl<'a, H: Q3RankingHost + 'a> ApplicationQ3Rankings<'a, H> {
    /// Build the match over a pool, level, provider, and host.
    pub fn new(
        pool: EntityPool,
        level: Rc<RefCell<GameLevel>>,
        provider: Option<&'a mut dyn RankingServiceProvider>,
        host: H,
    ) -> Result<Self, Q3RankingsError> {
        let pool = Rc::new(pool);
        let host = Rc::new(RefCell::new(host));
        let status_host = Rc::clone(&host);
        let service_host = Rc::clone(&host);
        let lifecycle = Rc::new(RefCell::new(RankingLifecycle::new(
            provider,
            move |slot, state| status_host.borrow_mut().status(slot, state),
            move |state| service_host.borrow_mut().service_status(state),
        )));
        let shared = Rc::new(RefCell::new(Shared {
            reports: Vec::new(),
            observed: HashMap::new(),
            weapons: HashMap::new(),
            closed: false,
            ended: false,
            accepting: false,
            overflowed: false,
        }));
        let sink_shared = Rc::clone(&shared);
        let sink: Rc<dyn Fn(Q3RankingReport)> = Rc::new(move |report| {
            let mut shared = sink_shared.borrow_mut();
            if !shared.accepting {
                return;
            }
            if shared.reports.len() >= 65536 {
                shared.reports.clear();
                shared.overflowed = true;
                return;
            }
            shared.reports.push(report);
        });
        let warmup_level = Rc::clone(&level);
        let warmup: Rc<dyn Fn() -> bool> = Rc::new(move || warmup_level.borrow().base.warmup_time != 0);
        let detach = pool.rankings.borrow().attach(sink, warmup)?;
        Ok(Self {
            pool,
            level,
            host,
            lifecycle,
            shared,
            detach,
        })
    }

    /// The shared lifecycle (donor `lifecycle`).
    #[must_use]
    pub fn lifecycle(&self) -> Ref<'_, RankingLifecycle<'a>> {
        self.lifecycle.borrow()
    }

    /// Reject work after close (donor `assertOpen`).
    fn assert_open(shared: &Shared) -> Result<(), Q3RankingsError> {
        if shared.closed {
            return Err(Q3RankingsError::Closed);
        }
        Ok(())
    }

    /// Sync the sink's accepting flag with the lifecycle (donor state check).
    fn sync_accepting(&self) {
        let accepting = matches!(self.lifecycle.borrow().state(), RankingServiceState::Active { .. })
            && !self.shared.borrow().ended;
        self.shared.borrow_mut().accepting = accepting;
    }

    /// Surface a sink overflow as unavailable (donor overflow branch).
    fn apply_overflow(&mut self) {
        if self.shared.borrow().overflowed {
            self.shared.borrow_mut().overflowed = false;
            self.lifecycle.borrow_mut().unavailable(
                "Ranking provider cannot keep up with source reports; this match cannot be submitted completely.",
            );
        }
        self.sync_accepting();
    }

    /// Reject account work without a connected client (donor `assertClient`).
    fn assert_client(&self, slot: i32) -> Result<(), Q3RankingsError> {
        Self::assert_open(&self.shared.borrow())?;
        if slot < 0
            || slot as usize >= self.pool.max_clients()
            || self.pool.get(slot).is_none_or(|entity| {
                let entity = entity.borrow();
                entity.client.is_none() || !entity.inuse
            })
        {
            return Err(Q3RankingsError::NoClient);
        }
        Ok(())
    }

    /// Begin the match (donor `begin`).
    pub fn begin(&mut self, enabled: bool, single_player: bool, game_key: &str) -> Result<(), Q3RankingsError> {
        Self::assert_open(&self.shared.borrow())?;
        self.lifecycle.borrow_mut().begin(enabled, single_player, game_key)?;
        self.sync_accepting();
        self.frame()
    }

    /// Sign a slot in (donor `account`).
    pub fn account(&mut self, slot: i32, request: RankingAccountRequest) -> Result<(), Q3RankingsError> {
        self.assert_client(slot)?;
        self.lifecycle.borrow_mut().account(slot, request)?;
        Ok(())
    }

    /// Account actions for one slot (donor `accountActions`).
    pub fn account_actions(&self, slot: i32) -> Result<Q3RankingAccountActions<'a>, Q3RankingsError> {
        self.assert_client(slot)?;
        Ok(Q3RankingAccountActions {
            lifecycle: Rc::clone(&self.lifecycle),
            shared: Rc::clone(&self.shared),
            pool: Rc::clone(&self.pool),
            slot,
        })
    }

    /// Reset a slot (donor `reset`).
    pub fn reset(&mut self, slot: i32) -> Result<(), Q3RankingsError> {
        self.assert_client(slot)?;
        self.lifecycle.borrow_mut().reset(slot)?;
        Ok(())
    }

    /// Move a slot to spectator (donor `spectate`).
    pub fn spectate(&mut self, slot: i32) -> Result<(), Q3RankingsError> {
        self.assert_client(slot)?;
        self.flush()?;
        self.lifecycle.borrow_mut().spectate(slot)?;
        Ok(())
    }

    /// Disconnect a slot (donor `disconnect`).
    pub fn disconnect(&mut self, slot: i32) -> Result<(), Q3RankingsError> {
        Self::assert_open(&self.shared.borrow())?;
        self.flush()?;
        self.lifecycle.borrow_mut().disconnect(slot)?;
        let mut shared = self.shared.borrow_mut();
        shared.observed.remove(&slot);
        shared.weapons.remove(&slot);
        Ok(())
    }

    /// Submit queued reports while active (donor `flush`).
    fn flush(&mut self) -> Result<(), Q3RankingsError> {
        self.apply_overflow();
        let reports = std::mem::take(&mut self.shared.borrow_mut().reports);
        for report in reports {
            if !matches!(self.lifecycle.borrow().state(), RankingServiceState::Active { .. }) {
                break;
            }
            match report {
                Q3RankingReport::Integer {
                    slf,
                    other,
                    key,
                    value,
                    accumulate,
                } => {
                    self.lifecycle
                        .borrow_mut()
                        .report_int(slf, other, key, value, accumulate)?;
                }
                Q3RankingReport::String { slf, other, key, value } => {
                    self.lifecycle.borrow_mut().report_string(slf, other, key, &value)?;
                }
            }
        }
        Ok(())
    }

    /// Poll the provider and reconcile every slot (donor `frame`).
    pub fn frame(&mut self) -> Result<(), Q3RankingsError> {
        Self::assert_open(&self.shared.borrow())?;
        self.flush()?;
        self.lifecycle.borrow_mut().frame()?;
        self.sync_accepting();
        if !matches!(self.lifecycle.borrow().state(), RankingServiceState::Active { .. }) || self.shared.borrow().ended
        {
            return Ok(());
        }
        for slot in 0..self.pool.max_clients() as i32 {
            let Some(entity) = self.pool.get(slot) else {
                continue;
            };
            let (weapon, team) = {
                let entity = entity.borrow();
                match &entity.client {
                    Some(client) if entity.inuse => (client.ps.weapon as i32, client.sess.session_team),
                    _ => continue,
                }
            };
            if entity.borrow().r.sv_flags & ServerEntityFlags::Bot as i32 != 0 {
                self.host.borrow_mut().drop_bot(slot);
                continue;
            }
            let time = self.level.borrow().base.time;
            let watch = self.shared.borrow().weapons.get(&slot).copied();
            match watch {
                None => {
                    self.shared
                        .borrow_mut()
                        .weapons
                        .insert(slot, Q3WeaponWatch { weapon, since: time });
                }
                Some(watch) if watch.weapon != weapon => {
                    self.pool
                        .rankings
                        .borrow()
                        .weapon_time(slot, watch.weapon, (time - watch.since) / 1000);
                    self.shared
                        .borrow_mut()
                        .weapons
                        .insert(slot, Q3WeaponWatch { weapon, since: time });
                }
                _ => {}
            }
            let state = self.lifecycle.borrow().player(slot);
            let previous = self.shared.borrow().observed.get(&slot).cloned();
            self.shared.borrow_mut().observed.insert(slot, state.clone());
            if previous.as_ref().map(std::mem::discriminant) != Some(std::mem::discriminant(&state)) {
                self.host.borrow_mut().status(slot, &state);
            }
            match &state {
                RankingPlayerState::New | RankingPlayerState::Spectator => {
                    if team != Team::TeamSpectator {
                        self.host.borrow_mut().spectator(slot);
                        self.host.borrow_mut().menu(slot);
                    }
                }
                RankingPlayerState::Pending => {}
                RankingPlayerState::Denied { .. } => {
                    self.lifecycle.borrow_mut().reset(slot)?;
                }
                RankingPlayerState::Active { .. } => {
                    if team == Team::TeamSpectator && self.host.borrow().game_type() < GameType::GtTeam as i32 {
                        self.host.borrow_mut().activate(slot);
                    }
                    if previous.as_ref().map(std::mem::discriminant) != Some(std::mem::discriminant(&state)) {
                        for other in 0..self.pool.max_clients() as i32 {
                            let Some(peer) = self.pool.get(other) else {
                                continue;
                            };
                            let peer = peer.borrow();
                            if !peer.inuse
                                || peer.client.is_none()
                                || peer.r.sv_flags & ServerEntityFlags::Bot as i32 != 0
                            {
                                continue;
                            }
                            if other != slot
                                && matches!(self.lifecycle.borrow().player(other), RankingPlayerState::Active { .. })
                            {
                                self.lifecycle
                                    .borrow_mut()
                                    .report_int(slot, other, 1210000002, 1, false)?;
                            }
                            self.host.borrow_mut().scoreboard(other);
                        }
                    }
                }
            }
        }
        let fraglimit = js_number(&self.host.borrow().cvar("fraglimit"));
        let timelimit = js_number(&self.host.borrow().cvar("timelimit"));
        if (fraglimit == 0.0 || fraglimit > 100.0) && (timelimit == 0.0 || timelimit > 1000.0) {
            self.host.borrow_mut().set_cvar("timelimit", "1000");
        }
        Ok(())
    }

    /// Submit the match report once (donor `gameOver`).
    pub fn game_over(&mut self) -> Result<(), Q3RankingsError> {
        Self::assert_open(&self.shared.borrow())?;
        if self.shared.borrow().ended || self.level.borrow().base.warmup_time != 0 {
            return Ok(());
        }
        self.flush()?;
        self.shared.borrow_mut().ended = true;
        self.sync_accepting();
        for (name, key) in [
            ("sv_hostname", 1000010000),
            ("mapname", 1000010001),
            ("fs_game", 1000010002),
            ("version", 1000010011),
        ] {
            let value = self.host.borrow().cvar(name);
            self.lifecycle.borrow_mut().report_string(-1, -1, key, &value)?;
        }
        for (name, key) in [
            ("g_gametype", 1010010003),
            ("fraglimit", 1010010004),
            ("timelimit", 1010010005),
            ("sv_maxclients", 1010010006),
            ("sv_maxRate", 1010010007),
            ("sv_minPing", 1010010008),
            ("sv_maxPing", 1010010009),
            ("dedicated", 1010010010),
        ] {
            let value = js_number(&self.host.borrow().cvar(name)).trunc() as i32;
            self.lifecycle.borrow_mut().report_int(-1, -1, key, value, false)?;
        }
        Ok(())
    }

    /// Detach and end the match (donor `close`).
    pub fn close(&mut self) -> Result<(), Q3RankingsError> {
        if self.shared.borrow().closed {
            return Ok(());
        }
        self.shared.borrow_mut().closed = true;
        (self.detach)();
        let flushed = self.flush();
        let ended = self.lifecycle.borrow_mut().end();
        flushed?;
        ended?;
        Ok(())
    }
}

/// Account actions for one slot (donor `accountActions` result).
///
/// The merged account trait speaks the UI-local mirrors, so service and player snapshots
/// are mapped from the authoritative lifecycle states.
pub struct Q3RankingAccountActions<'a> {
    lifecycle: Rc<RefCell<RankingLifecycle<'a>>>,
    shared: Rc<RefCell<Shared>>,
    pool: Rc<EntityPool>,
    slot: i32,
}

impl Q3RankingAccountActions<'_> {
    /// Reject work without a connected client.
    fn assert_client(&self) -> Result<(), String> {
        if self.shared.borrow().closed {
            return Err(Q3RankingsError::Closed.to_string());
        }
        if self.slot < 0
            || self.slot as usize >= self.pool.max_clients()
            || self.pool.get(self.slot).is_none_or(|entity| {
                let entity = entity.borrow();
                entity.client.is_none() || !entity.inuse
            })
        {
            return Err(Q3RankingsError::NoClient.to_string());
        }
        Ok(())
    }
}

impl RankingAccountActions for Q3RankingAccountActions<'_> {
    fn service(&self) -> RankingServiceView {
        match self.lifecycle.borrow().state() {
            RankingServiceState::Disabled => RankingServiceView::Disabled,
            RankingServiceState::Unavailable { reason } => RankingServiceView::Unavailable {
                message: reason.clone(),
            },
            RankingServiceState::Active { .. } => RankingServiceView::Active,
            RankingServiceState::Starting | RankingServiceState::Ending => RankingServiceView::Busy,
        }
    }

    fn player(&self) -> RankingPlayerView {
        match self.lifecycle.borrow().player(self.slot) {
            RankingPlayerState::New => RankingPlayerView::Idle,
            RankingPlayerState::Spectator => RankingPlayerView::Spectator,
            RankingPlayerState::Pending => RankingPlayerView::Pending,
            RankingPlayerState::Active { account } => RankingPlayerView::Active {
                account: RankingAccount {
                    player_id: account.player_id,
                    rank: account.rank,
                },
            },
            RankingPlayerState::Denied { reason } => RankingPlayerView::Denied { reason },
        }
    }

    fn submit(&mut self, request: UiAccountRequest) -> Result<(), String> {
        self.assert_client()?;
        let request = match request {
            UiAccountRequest::Login { username, password } => RankingAccountRequest::Login { username, password },
            UiAccountRequest::Create {
                username,
                password,
                email,
            } => RankingAccountRequest::Create {
                username,
                password,
                email,
            },
        };
        self.lifecycle
            .borrow_mut()
            .account(self.slot, request)
            .map_err(|error| error.to_string())
    }

    fn reset(&mut self) -> Result<(), String> {
        self.assert_client()?;
        self.lifecycle
            .borrow_mut()
            .reset(self.slot)
            .map_err(|error| error.to_string())
    }

    fn spectate(&mut self) -> Result<(), String> {
        self.assert_client()?;
        self.lifecycle
            .borrow_mut()
            .spectate(self.slot)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::q3::base::game::entities::EntityPoolOptions;
    use qa_content::q3::base::shared::definitions::Product;
    use qa_core::identity::IdentityOwner;
    use qa_net::services::rankings::{RankingLoginResult, RankingMatch};

    struct Stub {
        statuses: Vec<(i32, String)>,
        menus: Vec<i32>,
        cvars: HashMap<String, String>,
    }

    impl Q3RankingHost for Stub {
        fn status(&mut self, slot: i32, state: &RankingPlayerState) {
            let kind = match state {
                RankingPlayerState::New => "new",
                RankingPlayerState::Spectator => "spectator",
                RankingPlayerState::Pending => "pending",
                RankingPlayerState::Active { .. } => "active",
                RankingPlayerState::Denied { .. } => "denied",
            };
            self.statuses.push((slot, kind.to_string()));
        }
        fn menu(&mut self, slot: i32) {
            self.menus.push(slot);
        }
        fn spectator(&mut self, _slot: i32) {}
        fn activate(&mut self, _slot: i32) {}
        fn scoreboard(&mut self, _slot: i32) {}
        fn drop_bot(&mut self, _slot: i32) {}
        fn game_type(&self) -> i32 {
            GameType::GtFfa as i32
        }
        fn cvar(&self, name: &str) -> String {
            self.cvars.get(name).cloned().unwrap_or_default()
        }
        fn set_cvar(&mut self, name: &str, value: &str) {
            self.cvars.insert(name.to_string(), value.to_string());
        }
    }

    struct Provider;

    impl RankingServiceProvider for Provider {
        fn endpoint(&self) -> &str {
            "test"
        }
        fn begin(&mut self, _game_key: &str) -> Result<RankingMatch, RankingError> {
            Ok(RankingMatch { game_id: 1 })
        }
        fn login(
            &mut self,
            _game: &RankingMatch,
            _request: &RankingAccountRequest,
        ) -> Result<RankingLoginResult, RankingError> {
            Ok(RankingLoginResult::Active {
                account: qa_net::services::rankings::RankingAccount { player_id: 7, rank: 3 },
            })
        }
        fn logout(
            &mut self,
            _game: &RankingMatch,
            _account: &qa_net::services::rankings::RankingAccount,
        ) -> Result<(), RankingError> {
            Ok(())
        }
        fn finish(&mut self, _game: &RankingMatch) -> Result<(), RankingError> {
            Ok(())
        }
        fn poll(&mut self) -> Result<(), RankingError> {
            Ok(())
        }
        fn join(
            &mut self,
            _game: &RankingMatch,
            _account: &qa_net::services::rankings::RankingAccount,
        ) -> Result<(), RankingError> {
            Ok(())
        }
        fn report(
            &mut self,
            _game: &RankingMatch,
            _report: &qa_net::services::rankings::RankingServiceReport,
        ) -> Result<(), RankingError> {
            Ok(())
        }
    }

    fn harness() -> (ApplicationQ3Rankings<'static, Stub>, IdentityOwner) {
        let owner = IdentityOwner::create("q3-rankings").unwrap();
        let pool = EntityPool::open(EntityPoolOptions {
            product: Product::Baseq3,
            max_clients: 4,
            map_start_time: 0,
            time: Rc::new(|| 0),
            print: Rc::new(|_| {}),
            link: Rc::new(|_| {}),
            unlink: Rc::new(|_| {}),
            event_debug: None,
        });
        let level = Rc::new(RefCell::new(GameLevel::new()));
        let host = Stub {
            statuses: Vec::new(),
            menus: Vec::new(),
            cvars: HashMap::new(),
        };
        let provider: &'static mut dyn RankingServiceProvider = Box::leak(Box::new(Provider));
        let rankings = ApplicationQ3Rankings::new(pool, level, Some(provider), host).unwrap();
        (rankings, owner)
    }

    #[test]
    fn begin_runs_a_frame_and_clamps_timelimit() {
        let (mut rankings, _) = harness();
        rankings.begin(true, false, "key").unwrap();
        assert!(matches!(
            rankings.lifecycle().state(),
            RankingServiceState::Active { .. }
        ));
        assert_eq!(
            rankings.host.borrow().cvars.get("timelimit").map(String::as_str),
            Some("1000")
        );
    }

    #[test]
    fn frame_observes_connected_slots() {
        let (mut rankings, _) = harness();
        rankings.begin(true, false, "key").unwrap();
        {
            let entity = rankings.pool.get(0).unwrap();
            let mut entity = entity.borrow_mut();
            entity.inuse = true;
            entity.client.as_mut().unwrap().sess.session_team = Team::TeamRed;
        }
        rankings.frame().unwrap();
        let host = rankings.host.borrow();
        assert!(host.statuses.iter().any(|(slot, kind)| *slot == 0 && kind == "new"));
        assert!(host.menus.contains(&0));
    }

    #[test]
    fn account_actions_submit_and_reset() {
        let (mut rankings, _) = harness();
        rankings.begin(true, false, "key").unwrap();
        {
            let entity = rankings.pool.get(1).unwrap();
            entity.borrow_mut().inuse = true;
        }
        let mut actions = rankings.account_actions(1).unwrap();
        assert_eq!(actions.service(), RankingServiceView::Active);
        actions
            .submit(UiAccountRequest::Login {
                username: "a".to_string(),
                password: "b".to_string(),
            })
            .unwrap();
        assert!(matches!(actions.player(), RankingPlayerView::Active { .. }));
        actions.reset().unwrap();
        assert!(rankings.account_actions(9).is_err());
    }

    #[test]
    fn game_over_reports_once() {
        let (mut rankings, _) = harness();
        rankings.begin(true, false, "key").unwrap();
        rankings.game_over().unwrap();
        rankings.game_over().unwrap();
        rankings.close().unwrap();
        assert!(rankings.frame().is_err());
    }

    #[test]
    fn sink_overflow_marks_provider_unavailable() {
        let (mut rankings, _) = harness();
        rankings.begin(true, false, "key").unwrap();
        for _ in 0..65537 {
            rankings.pool.rankings.borrow().weapon_time(0, 1, 5);
        }
        rankings.frame().unwrap();
        assert!(matches!(
            rankings.lifecycle().state(),
            RankingServiceState::Unavailable { .. }
        ));
    }

    #[test]
    fn disconnect_forgets_slot() {
        let (mut rankings, _) = harness();
        rankings.begin(true, false, "key").unwrap();
        {
            let entity = rankings.pool.get(0).unwrap();
            entity.borrow_mut().inuse = true;
        }
        rankings.frame().unwrap();
        rankings.disconnect(0).unwrap();
        assert!(!rankings.shared.borrow().observed.contains_key(&0));
    }
}
