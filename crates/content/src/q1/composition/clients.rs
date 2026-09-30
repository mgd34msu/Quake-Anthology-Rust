//! Q1 source clients (`src/content/composition/q1/clients.ts`).
//!
//! Native client globals on the same source actors as the selected
//! character. Records key on the actor id; the release hook deletes
//! them synchronously, so record presence is equivalent to the
//! donor's owned-actor resolution.

use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};

use crate::q1::composition::types::{
    Q1ClientAdmission, Q1ClientSnapshot, Q1CompositionEvent, Q1CompositionServices, Q1SourceProgram,
};
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::CombatTraits;
use crate::q1::foundation::types::{Q1AutoSwitch, Q1Edition};
use crate::q1::{q1_error, Q1Error};
use crate::value::{arr, boolean, int, num, obj, str, SaveReader};

/// Admitted source client (`Q1SourceClient`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SourceClient {
    /// Owning actor.
    pub actor: OwnedActor,
    /// Native client slot.
    pub slot: i32,
    /// Userinfo pairs in donor order.
    pub userinfo: Vec<(String, String)>,
    /// Frag count.
    pub frags: f64,
    /// Team number.
    pub team: i32,
    /// Whether the client observes.
    pub observer: bool,
    /// Whether the client is untargetable.
    pub no_target: bool,
    /// God mode.
    pub god_mode: bool,
    /// Pending impulse.
    pub impulse: i32,
    /// Use held.
    pub use_action: bool,
    /// Whether death was recorded.
    pub death_recorded: bool,
    /// Respawn request time in seconds (`-1` when idle).
    pub respawn_requested_at: f64,
}

impl Q1SourceClient {
    /// Fresh client.
    pub fn new(actor: OwnedActor, slot: i32, userinfo: Vec<(String, String)>) -> Self {
        Self {
            actor,
            slot,
            userinfo,
            frags: 0.0,
            team: 0,
            observer: false,
            no_target: false,
            god_mode: false,
            impulse: 0,
            use_action: false,
            death_recorded: false,
            respawn_requested_at: -1.0,
        }
    }

    /// Userinfo value, if present.
    fn value(&self, key: &str) -> Option<&str> {
        self.userinfo
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// Display name (`"unconnected"` when unset).
    #[must_use]
    pub fn name(&self) -> String {
        let name = self.value("name").unwrap_or("");
        if name.is_empty() {
            String::from("unconnected")
        } else {
            name.to_string()
        }
    }

    /// Shirt color.
    #[must_use]
    pub fn shirt(&self) -> i32 {
        color(self.value("topcolor"))
    }

    /// Pants color.
    #[must_use]
    pub fn pants(&self) -> i32 {
        color(self.value("bottomcolor"))
    }

    /// Publishable snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Q1ClientSnapshot {
        Q1ClientSnapshot {
            actor: self.actor.id().clone(),
            slot: self.slot,
            name: self.name(),
            frags: self.frags,
            shirt: self.shirt(),
            pants: self.pants(),
            team: self.team,
            observer: self.observer,
            no_target: self.no_target,
            userinfo: self.userinfo.clone(),
        }
    }
}

/// Parse a donor `Number` color clamped to `0..=13`.
fn color(value: Option<&str>) -> i32 {
    let text = value.unwrap_or("0").trim();
    let parsed = text.parse::<f64>().ok().or_else(|| {
        for (prefix, radix) in [("0x", 16), ("0X", 16), ("0b", 2), ("0B", 2), ("0o", 8), ("0O", 8)] {
            if let Some(digits) = text.strip_prefix(prefix) {
                if digits.is_empty() {
                    return None;
                }
                return i64::from_str_radix(digits, radix).ok().map(|parsed| parsed as f64);
            }
        }
        None
    });
    let parsed = parsed.unwrap_or(0.0);
    if !parsed.is_finite() {
        return 0;
    }
    parsed.trunc().clamp(0.0, 13.0) as i32
}

/// Cached held input mirroring the player state. The composition is
/// the only production writer of `attack_held`/`jump_held` (through
/// `player_input`), so the cache is exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Q1HeldInput {
    /// Attack held.
    pub attack: bool,
    /// Jump held.
    pub jump: bool,
}

/// Native client globals (`Q1SourceClients`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q1SourceClients {
    /// Admitted clients by actor.
    pub records: HashMap<ActorId, Q1SourceClient>,
    /// Red captures.
    pub red_captures: i32,
    /// Blue captures.
    pub blue_captures: i32,
    /// Cached held input by actor.
    pub held: HashMap<ActorId, Q1HeldInput>,
}

impl Q1SourceClients {
    /// Fresh client table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a client record.
    #[must_use]
    pub fn client(&self, actor: &ActorId) -> Option<&Q1SourceClient> {
        self.records.get(actor)
    }

    /// Read a client record or fail with the donor message.
    pub fn require(&self, actor: &ActorId) -> Result<&Q1SourceClient, Q1Error> {
        self.client(actor)
            .ok_or_else(|| q1_error("Q1 source client is not admitted"))
    }

    /// Read a mutable client record or fail with the donor message.
    pub fn require_mut(&mut self, actor: &ActorId) -> Result<&mut Q1SourceClient, Q1Error> {
        self.records
            .get_mut(actor)
            .ok_or_else(|| q1_error("Q1 source client is not admitted"))
    }

    /// Drop a client record and its cached input.
    pub fn remove(&mut self, actor: &ActorId) {
        self.records.remove(actor);
        self.held.remove(actor);
    }

    /// Admit a client (`attach`).
    pub fn attach(
        &mut self,
        game: &mut Q1EntityServices,
        services: &mut dyn Q1CompositionServices,
        program: Q1SourceProgram,
        actor: &OwnedActor,
        admission: &Q1ClientAdmission,
    ) -> Result<(), Q1Error> {
        game.host.actors.assert_owned(actor)?;
        if self.records.contains_key(actor.id()) || self.records.values().any(|client| client.slot == admission.slot) {
            return Err(q1_error("Q1 source client slot is already admitted"));
        }
        let max_clients = game.options().max_clients.unwrap_or(1);
        if admission.slot < 0 || admission.slot >= max_clients {
            return Err(q1_error("Q1 source client slot is out of range"));
        }
        let mut client = Q1SourceClient::new(actor.clone(), admission.slot, admission.userinfo.clone());
        client.team = client.pants() + 1;
        self.records.insert(actor.id().clone(), client);
        self.apply_player_settings(game, program, actor.id())?;
        self.publish(services, actor.id())
    }

    /// Replace userinfo (`update`).
    pub fn update(
        &mut self,
        game: &mut Q1EntityServices,
        services: &mut dyn Q1CompositionServices,
        program: Q1SourceProgram,
        actor: &ActorId,
        userinfo: &[(String, String)],
    ) -> Result<(), Q1Error> {
        let previous = self.require(actor)?.pants();
        let client = self.require_mut(actor)?;
        client.userinfo = userinfo.to_vec();
        if client.pants() != previous {
            client.team = client.pants() + 1;
        }
        self.apply_player_settings(game, program, actor)?;
        self.publish(services, actor)
    }

    /// Write shirt and pants colors plus the derived team, without
    /// touching the game. Deferred settings application follows
    /// through [`Q1SourceClients::apply_player_settings`].
    pub fn set_colors_data(&mut self, actor: &ActorId, shirt: i32, pants: i32) -> Result<(), Q1Error> {
        let shirt_text = shirt.to_string();
        let pants_text = pants.to_string();
        let client = self.require_mut(actor)?;
        let shirt = color(Some(shirt_text.as_str())).to_string();
        let pants = color(Some(pants_text.as_str())).to_string();
        set_userinfo(&mut client.userinfo, "topcolor", shirt.as_str());
        set_userinfo(&mut client.userinfo, "bottomcolor", pants.as_str());
        let pants = client.pants();
        client.team = pants + 1;
        Ok(())
    }

    /// Set shirt and pants colors (`colors`).
    pub fn colors(
        &mut self,
        game: &mut Q1EntityServices,
        services: &mut dyn Q1CompositionServices,
        program: Q1SourceProgram,
        actor: &ActorId,
        shirt: i32,
        pants: i32,
    ) -> Result<(), Q1Error> {
        self.set_colors_data(actor, shirt, pants)?;
        self.apply_player_settings(game, program, actor)?;
        self.publish(services, actor)
    }

    /// Team color (`teamColor`).
    pub fn team_color(&self, actor: &ActorId) -> Result<i32, Q1Error> {
        Ok(self.require(actor)?.team)
    }

    /// Note a spawn (`spawned`).
    pub fn spawned(
        &mut self,
        game: &mut Q1EntityServices,
        services: &mut dyn Q1CompositionServices,
        program: Q1SourceProgram,
        actor: &ActorId,
    ) -> Result<(), Q1Error> {
        let client = self.require_mut(actor)?;
        client.god_mode = false;
        if game.options().edition == Q1Edition::Rerelease && program != Q1SourceProgram::Ctf {
            if game.options().coop {
                client.team = 1;
            } else if program == Q1SourceProgram::Id1 {
                client.team = -1;
            }
        }
        self.apply_player_settings(game, program, actor)?;
        self.publish(services, actor)
    }

    /// Apply combat traits and the autoswitch policy
    /// (`applyPlayerSettings`).
    pub fn apply_player_settings(
        &self,
        game: &mut Q1EntityServices,
        program: Q1SourceProgram,
        actor: &ActorId,
    ) -> Result<(), Q1Error> {
        let client = self.require(actor)?;
        let team = if program == Q1SourceProgram::Ctf {
            if client.team == 5 {
                Some(String::from("red"))
            } else if client.team == 14 {
                Some(String::from("blue"))
            } else {
                None
            }
        } else if client.team > 0 {
            Some(client.team.to_string())
        } else {
            None
        };
        let combat = game
            .host
            .combat
            .read(actor)
            .ok_or_else(|| q1_error("Actor has no combat binding"))?;
        game.host.combat.set_traits(
            &client.actor,
            CombatTraits {
                can_take_damage: combat.can_take_damage,
                mass: combat.mass,
                invulnerable: combat.invulnerable,
                team,
                no_knockback: combat.no_knockback,
            },
        )?;
        let auto_switch = client.value("qts_weapon_autoswitch").map(|policy| policy.to_string());
        if game.player_ref(actor).is_some() {
            if let Some(policy) = auto_switch {
                let policy = match policy.as_str() {
                    "new" => Q1AutoSwitch::New,
                    "never" => Q1AutoSwitch::Never,
                    _ => Q1AutoSwitch::Always,
                };
                game.update_player(actor, |player| player.auto_switch = policy)?;
            }
        }
        Ok(())
    }

    /// Adjust the score (`addScore`).
    pub fn add_score(
        &mut self,
        services: &mut dyn Q1CompositionServices,
        actor: &ActorId,
        delta: f64,
    ) -> Result<(), Q1Error> {
        let client = self.require_mut(actor)?;
        client.frags = f64::from((client.frags + delta) as f32);
        self.publish(services, actor)
    }

    /// Set observer mode (`setObserver`).
    pub fn set_observer(
        &mut self,
        services: &mut dyn Q1CompositionServices,
        actor: &ActorId,
        observer: bool,
    ) -> Result<(), Q1Error> {
        self.require_mut(actor)?.observer = observer;
        services.set_observer(actor, observer);
        self.publish(services, actor)
    }

    /// Publish a client snapshot (`publish`).
    pub fn publish(&self, services: &mut dyn Q1CompositionServices, actor: &ActorId) -> Result<(), Q1Error> {
        let snapshot = self.require(actor)?.snapshot();
        services.emit(Q1CompositionEvent::Client(snapshot));
        Ok(())
    }

    /// Cache held input mirroring the player state.
    pub fn record_input(&mut self, actor: &ActorId, attack: bool, jump: bool) {
        self.held.insert(actor.clone(), Q1HeldInput { attack, jump });
    }

    /// Cached held input, defaulting to released.
    #[must_use]
    pub fn held_input(&self, actor: &ActorId) -> Q1HeldInput {
        self.held.get(actor).copied().unwrap_or_default()
    }

    /// Capture the checkpoint value (`capture`).
    #[must_use]
    pub fn capture(&self, program: Q1SourceProgram) -> Vec<u8> {
        let mut clients: Vec<&Q1SourceClient> = self.records.values().collect();
        clients.sort_by(|left, right| {
            left.slot
                .cmp(&right.slot)
                .then_with(|| left.actor.id().slot().cmp(&right.actor.id().slot()))
                .then_with(|| left.actor.id().generation().cmp(&right.actor.id().generation()))
        });
        encode_checkpoint_value(&obj(vec![
            ("version", int(1)),
            ("program", str(program.as_str())),
            ("redCaptures", int(i64::from(self.red_captures))),
            ("blueCaptures", int(i64::from(self.blue_captures))),
            (
                "clients",
                arr(clients
                    .into_iter()
                    .map(|client| {
                        obj(vec![
                            (
                                "actor",
                                obj(vec![
                                    ("slot", int(i64::from(client.actor.id().slot()))),
                                    ("generation", int(i64::from(client.actor.id().generation()))),
                                ]),
                            ),
                            ("slot", int(i64::from(client.slot))),
                            (
                                "userinfo",
                                arr(client
                                    .userinfo
                                    .iter()
                                    .map(|(key, value)| obj(vec![("key", str(key)), ("value", str(value))]))
                                    .collect()),
                            ),
                            ("frags", num(client.frags)),
                            ("team", num(f64::from(client.team))),
                            ("observer", boolean(client.observer)),
                            ("noTarget", boolean(client.no_target)),
                            ("godMode", boolean(client.god_mode)),
                            ("impulse", int(i64::from(client.impulse))),
                            ("use", boolean(client.use_action)),
                            ("deathRecorded", boolean(client.death_recorded)),
                            ("respawnRequestedAt", num(client.respawn_requested_at)),
                        ])
                    })
                    .collect()),
            ),
        ]))
    }

    /// Restore the checkpoint value (`restore`).
    pub fn restore(
        &mut self,
        game: &mut Q1EntityServices,
        program: Q1SourceProgram,
        bytes: &[u8],
    ) -> Result<(), Q1Error> {
        let value = decode_checkpoint_value(bytes)?;
        let reader = SaveReader::at(&value, "q1:source-clients");
        reader.field("version").literal_i64(1)?;
        reader.field("program").literal_str(program.as_str())?;
        let red_captures = reader.field("redCaptures").integer(0)?;
        let blue_captures = reader.field("blueCaptures").integer(0)?;
        let mut records: HashMap<ActorId, Q1SourceClient> = HashMap::new();
        reader.field("clients").list(|saved| {
            let id = saved.field("actor");
            let actor = game
                .host
                .actors
                .resolve_saved(&SavedActorId {
                    slot: u32::try_from(id.field("slot").integer(0)?).unwrap_or(u32::MAX),
                    generation: u32::try_from(id.field("generation").integer(0)?).unwrap_or(u32::MAX),
                })
                .ok_or_else(|| Q1Error::from(id.fail("missing source client actor")))?;
            let slot = saturate_i32(saved.field("slot").integer(0)?);
            if records.values().any(|client: &Q1SourceClient| client.slot == slot) {
                return Err(Q1Error::from(saved.fail("duplicate source client slot")));
            }
            let mut userinfo = Vec::new();
            saved.field("userinfo").list(|entry| {
                userinfo.push((entry.field("key").string()?, entry.field("value").string()?));
                Ok::<(), Q1Error>(())
            })?;
            let mut client = Q1SourceClient::new(actor.clone(), slot, userinfo);
            client.frags = saved.field("frags").finite()?;
            client.observer = saved.field("observer").boolean()?;
            client.impulse = saturate_i32(saved.field("impulse").integer(0)?);
            client.use_action = saved.field("use").boolean()?;
            client.death_recorded = saved.field("deathRecorded").boolean()?;
            client.respawn_requested_at = saved.field("respawnRequestedAt").finite()?;
            client.no_target = saved.field("noTarget").boolean()?;
            let god_mode = saved.field("godMode");
            client.god_mode = if god_mode.value.is_none() {
                false
            } else {
                god_mode.boolean()?
            };
            client.team = saved.field("team").finite()? as i32;
            records.insert(actor.id().clone(), client);
            Ok::<(), Q1Error>(())
        })?;
        self.red_captures = saturate_i32(red_captures);
        self.blue_captures = saturate_i32(blue_captures);
        self.records = records;
        self.held.clear();
        Ok(())
    }
}

/// Saturate a checkpoint integer into an `i32`.
fn saturate_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Replace a userinfo pair, preserving donor order.
fn set_userinfo(userinfo: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some(entry) = userinfo.iter_mut().rev().find(|(name, _)| name == key) {
        entry.1 = value.to_string();
    } else {
        userinfo.push((key.to_string(), value.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::composition::types::FakeCompositionServices;
    use crate::q1::missionpacks::types::test_game;

    fn admission(slot: i32) -> Q1ClientAdmission {
        Q1ClientAdmission {
            slot,
            userinfo: vec![
                (String::from("name"), String::from("Player")),
                (String::from("topcolor"), String::from("2")),
                (String::from("bottomcolor"), String::from("4")),
            ],
        }
    }

    fn admitted() -> (Q1EntityServices, Q1SourceClients, FakeCompositionServices, OwnedActor) {
        let mut game = test_game();
        let mut clients = Q1SourceClients::new();
        let mut services = FakeCompositionServices::new();
        let actor = game
            .host
            .actors
            .allocate_at_source(&qa_core::identity::ProviderId::new("q1", "test"), 1, "q1:player")
            .expect("actor");
        game.host
            .combat
            .create(
                &actor,
                &crate::q1::foundation::gameplay::CombatState {
                    health: 100.0,
                    armor: crate::contract::ArmorState {
                        regular: crate::contract::RegularArmorState::None,
                        powered: crate::contract::PoweredProtectionState::None,
                    },
                    mass: 200.0,
                    can_take_damage: true,
                    invulnerable: false,
                    no_knockback: None,
                    team: None,
                },
            )
            .expect("combat");
        clients
            .attach(&mut game, &mut services, Q1SourceProgram::Id1, &actor, &admission(0))
            .expect("attach");
        (game, clients, services, actor)
    }

    #[test]
    fn attach_admits_and_publishes() {
        let (_game, clients, services, actor) = admitted();
        let client = clients.require(actor.id()).expect("client");
        assert_eq!(client.slot, 0);
        assert_eq!(client.name(), "Player");
        assert_eq!(client.shirt(), 2);
        assert_eq!(client.pants(), 4);
        assert_eq!(client.team, 5);
        assert!(matches!(
            services.events.as_slice(),
            [Q1CompositionEvent::Client(snapshot)] if snapshot.name == "Player" && snapshot.team == 5
        ));
    }

    #[test]
    fn attach_rejects_duplicates_and_range() {
        let (mut game, mut clients, mut services, actor) = admitted();
        let duplicate = admission(0);
        assert!(clients
            .attach(&mut game, &mut services, Q1SourceProgram::Id1, &actor, &duplicate)
            .is_err());
        let other = game
            .host
            .actors
            .allocate_at_source(&qa_core::identity::ProviderId::new("q1", "test"), 2, "q1:player")
            .expect("other");
        game.host
            .combat
            .create(
                &other,
                &crate::q1::foundation::gameplay::CombatState {
                    health: 100.0,
                    armor: crate::contract::ArmorState {
                        regular: crate::contract::RegularArmorState::None,
                        powered: crate::contract::PoweredProtectionState::None,
                    },
                    mass: 200.0,
                    can_take_damage: true,
                    invulnerable: false,
                    no_knockback: None,
                    team: None,
                },
            )
            .expect("combat");
        let ranged = admission(99);
        let error = clients
            .attach(&mut game, &mut services, Q1SourceProgram::Id1, &other, &ranged)
            .expect_err("range");
        assert_eq!(error.to_string(), "Q1 source client slot is out of range");
    }

    #[test]
    fn colors_clamp_and_reteam() {
        let (mut game, mut clients, mut services, actor) = admitted();
        clients
            .colors(&mut game, &mut services, Q1SourceProgram::Id1, actor.id(), 99, -3)
            .expect("colors");
        let client = clients.require(actor.id()).expect("client");
        assert_eq!((client.shirt(), client.pants(), client.team), (13, 0, 1));
        let combat = game.host.combat.read(actor.id()).expect("combat");
        assert_eq!(combat.team.as_deref(), Some("1"));
    }

    #[test]
    fn capture_restore_round_trips() {
        let (mut game, mut clients, mut services, actor) = admitted();
        clients.red_captures = 2;
        clients.blue_captures = 3;
        clients.add_score(&mut services, actor.id(), 5.0).expect("score");
        let bytes = clients.capture(Q1SourceProgram::Id1);
        let mut restored = Q1SourceClients::new();
        restored
            .restore(&mut game, Q1SourceProgram::Id1, &bytes)
            .expect("restore");
        assert_eq!(restored.red_captures, 2);
        assert_eq!(restored.blue_captures, 3);
        let client = restored.require(actor.id()).expect("client");
        assert_eq!(client.frags, 5.0);
        assert_eq!(client.name(), "Player");
        let again = restored.capture(Q1SourceProgram::Id1);
        assert_eq!(bytes, again);
    }

    #[test]
    fn restore_rejects_program_mismatch() {
        let (mut game, clients, _services, _actor) = admitted();
        let bytes = clients.capture(Q1SourceProgram::Id1);
        let mut restored = Q1SourceClients::new();
        assert!(restored.restore(&mut game, Q1SourceProgram::Ctf, &bytes).is_err());
    }

    #[test]
    fn color_matches_donor() {
        assert_eq!(color(None), 0);
        assert_eq!(color(Some("7")), 7);
        assert_eq!(color(Some("99")), 13);
        assert_eq!(color(Some("-2")), 0);
        assert_eq!(color(Some("nope")), 0);
        assert_eq!(color(Some("0x10")), 13);
        assert_eq!(color(Some("unconnected")), 0);
    }
}
