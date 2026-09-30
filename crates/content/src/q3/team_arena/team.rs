//! Quake III team-arena: team.
//!
//! Donor provenance: `src/content/q3/team-arena/team.ts`.

use qa_core::math::{dot3, length3, sub3, vec3, Vec3};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::format::{game_format, game_format_bounded, GameFormatArgument};
use crate::q3::base::game::state::{ConnectionState, GameFlags, MAX_CLIENTS};
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::entity_shared::ServerEntityFlags;
use crate::q3::team_arena::support::*;

// ---------------------------------------------------------------------------
// Pattern-position aliases for Team discriminants (`as` casts are not patterns).
const TEAM_FREE: i32 = Team::TeamFree as i32;
const TEAM_RED: i32 = Team::TeamRed as i32;
const TEAM_BLUE: i32 = Team::TeamBlue as i32;
const TEAM_SPECTATOR: i32 = Team::TeamSpectator as i32;

// team.ts
// ---------------------------------------------------------------------------

/// Flag status codes (`FlagStatus`).
pub mod flag_status {
    /// At base.
    pub const AT_BASE: i32 = 0;
    /// Taken.
    pub const TAKEN: i32 = 1;
    /// Taken by red.
    pub const TAKEN_RED: i32 = 2;
    /// Taken by blue.
    pub const TAKEN_BLUE: i32 = 3;
    /// Dropped.
    pub const DROPPED: i32 = 4;
}

/// Global team sounds (`GlobalTeamSound`).
pub mod global_team_sound {
    pub const RED_CAPTURE: i32 = 0;
    pub const BLUE_CAPTURE: i32 = 1;
    pub const RED_RETURN: i32 = 2;
    pub const BLUE_RETURN: i32 = 3;
    pub const RED_TAKEN: i32 = 4;
    pub const BLUE_TAKEN: i32 = 5;
    pub const RED_OBELISK_ATTACKED: i32 = 6;
    pub const BLUE_OBELISK_ATTACKED: i32 = 7;
    pub const RED_SCORED: i32 = 8;
    pub const BLUE_SCORED: i32 = 9;
    pub const RED_TOOK_LEAD: i32 = 10;
    pub const BLUE_TOOK_LEAD: i32 = 11;
    pub const TIED: i32 = 12;
    pub const KAMIKAZE: i32 = 13;
}

/// Obelisk tuning (`ObeliskSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObeliskSettings {
    /// Maximum health.
    pub health: i32,
    /// Regeneration period in seconds.
    pub regen_period_seconds: i32,
    /// Regeneration amount.
    pub regen_amount: i32,
    /// Respawn delay in seconds.
    pub respawn_delay_seconds: i32,
}

/// Team services (`TeamHost`).
pub trait TeamHost {
    /// Product.
    fn product(&self) -> Product;
    /// Entity pool.
    fn pool(&self) -> PoolRef;
    /// Server world.
    fn world(&self) -> WorldRef;
    /// Game type code.
    fn game_type(&self) -> i32;
    /// Current time.
    fn time(&self) -> i32;
    /// Shared team scores.
    fn team_scores(&self) -> SharedSlots;
    /// Rank-sorted clients.
    fn sorted_clients(&self) -> Vec<i32>;
    /// Location chain head.
    fn location_head(&self) -> Option<EntityRef>;
    /// Obelisk tuning (missionpack only).
    fn obelisk_settings(&self) -> Option<ObeliskSettings>;
    /// Send a server command.
    fn send_server_command(&self, client_num: i32, text: &str);
    /// Write a config string.
    fn set_configstring(&self, index: i32, text: &str);
    /// Warn a line.
    fn warn(&self, text: &str);
    /// Award score to a player.
    fn add_score(&self, player: &EntityRef, origin: Vec3, score: i32);
    /// Recalculate ranks.
    fn calculate_ranks(&self);
    /// Respawn an item.
    fn respawn_item(&self, item: &EntityRef);
    /// PVS visibility test.
    fn in_pvs(&self, first: Vec3, second: Vec3) -> bool;
}

/// Team game state (`TeamGameState`).
#[derive(Debug, Clone, PartialEq)]
pub struct TeamGameState {
    /// Last flag capture time.
    pub last_flag_capture: f32,
    /// Last capturing team.
    pub last_capture_team: i32,
    /// Red flag status.
    pub red_status: i32,
    /// Blue flag status.
    pub blue_status: i32,
    /// Neutral flag status.
    pub flag_status: i32,
    /// Red taken time.
    pub red_taken_time: i32,
    /// Blue taken time.
    pub blue_taken_time: i32,
    /// Red obelisk attacked time.
    pub red_obelisk_attacked_time: i32,
    /// Blue obelisk attacked time.
    pub blue_obelisk_attacked_time: i32,
}

impl Default for TeamGameState {
    fn default() -> Self {
        Self {
            last_flag_capture: 0.0,
            last_capture_team: 0,
            red_status: flag_status::AT_BASE,
            blue_status: flag_status::AT_BASE,
            flag_status: flag_status::AT_BASE,
            red_taken_time: 0,
            blue_taken_time: 0,
            red_obelisk_attacked_time: 0,
            blue_obelisk_attacked_time: 0,
        }
    }
}

/// Opposing team (`otherTeam`).
#[must_use]
pub fn other_team(team_code: i32) -> i32 {
    if team_code == Team::TeamRed as i32 {
        Team::TeamBlue as i32
    } else if team_code == Team::TeamBlue as i32 {
        Team::TeamRed as i32
    } else {
        team_code
    }
}

/// Team display name (`teamName`).
#[must_use]
pub fn team_name(team_code: i32) -> &'static str {
    if team_code == Team::TeamRed as i32 {
        "RED"
    } else if team_code == Team::TeamBlue as i32 {
        "BLUE"
    } else if team_code == Team::TeamSpectator as i32 {
        "SPECTATOR"
    } else {
        "FREE"
    }
}

/// Opposing team display name (`otherTeamName`).
#[must_use]
pub fn other_team_name(team_code: i32) -> &'static str {
    team_name(other_team(team_code))
}

/// Team color string (`teamColorString`).
#[must_use]
pub fn team_color_string(team_code: i32) -> &'static str {
    if team_code == Team::TeamRed as i32 {
        "^1"
    } else if team_code == Team::TeamBlue as i32 {
        "^4"
    } else if team_code == Team::TeamSpectator as i32 {
        "^3"
    } else {
        "^7"
    }
}

/// Whether two clients share a team (`onSameTeam`).
#[must_use]
pub fn on_same_team(game_type: i32, first: &EntityRef, second: &EntityRef) -> bool {
    let a = first.borrow().client.clone();
    let b = second.borrow().client.clone();
    match (a, b) {
        (Some(a), Some(b)) => {
            game_type >= GameType::GtTeam as i32 && a.borrow().sess.session_team == b.borrow().sess.session_team
        }
        _ => false,
    }
}

/// Team spawn-point bodies are empty in the source (`spawnTeamPoint`).
pub fn spawn_team_point(_entity: &EntityRef) {}

pub(crate) const TEAM_AWARDS: i32 = 0x8 | 0x40 | 0x800 | 0x8000 | 0x10000 | 0x20000;

pub(crate) fn team_client_of(entity: &EntityRef) -> ClientRef {
    match entity.borrow().client.clone() {
        Some(client) => client,
        None => panic!("Team player operation requires a client"),
    }
}

pub(crate) fn flag_team(entity: &EntityRef) -> Option<i32> {
    let item = match entity.borrow().item.clone() {
        Some(item) => item,
        None => panic!("Team item operation requires an item"),
    };
    if item.tag == Powerup::PwRedflag as i32 {
        Some(Team::TeamRed as i32)
    } else if item.tag == Powerup::PwBlueflag as i32 {
        Some(Team::TeamBlue as i32)
    } else if item.tag == Powerup::PwNeutralflag as i32 {
        Some(Team::TeamFree as i32)
    } else {
        None
    }
}

pub(crate) fn obelisk_health_fraction(health: i32, maximum: i32) -> i32 {
    // The source divides in float64; the zero-maximum case is a degenerate
    // configuration whose infinite result cannot be stored.
    if maximum == 0 {
        0
    } else {
        health.wrapping_mul(255) / maximum
    }
}

pub(crate) fn ctf_status(status: i32) -> String {
    match status {
        -1 => "\0".to_string(),
        flag_status::AT_BASE => "0".to_string(),
        flag_status::TAKEN => "1".to_string(),
        flag_status::TAKEN_RED | flag_status::TAKEN_BLUE => "*".to_string(),
        flag_status::DROPPED => "2".to_string(),
        _ => panic!("invalid flag status {status}"),
    }
}

pub(crate) struct TeamInner {
    host: Rc<dyn TeamHost>,
    state: RefCell<TeamGameState>,
    neutral_obelisk: RefCell<Option<EntityRef>>,
    last_team_location_time: Cell<i32>,
}

/// Team runtime (`TeamRuntime`).
pub struct TeamRuntime {
    inner: Rc<TeamInner>,
    obelisk_regen: ThinkCallback,
    obelisk_respawn: ThinkCallback,
    obelisk_die: DieCallback,
    obelisk_touch: TouchCallback,
    obelisk_pain: PainCallback,
}

impl TeamRuntime {
    /// Fresh runtime.
    pub fn new(host: Rc<dyn TeamHost>) -> Self {
        if host.team_scores().len() != 4 {
            panic!("Team scores require four source team slots");
        }
        let inner = Rc::new(TeamInner {
            host,
            state: RefCell::new(TeamGameState::default()),
            neutral_obelisk: RefCell::new(None),
            last_team_location_time: Cell::new(0),
        });
        let regen_inner = inner.clone();
        let obelisk_regen: ThinkCallback = Rc::new(move |entity: &EntityRef| {
            let runtime = TeamRuntime::wrap(regen_inner.clone());
            runtime.run_obelisk_regen(entity);
        });
        let respawn_inner = inner.clone();
        let obelisk_respawn: ThinkCallback = Rc::new(move |entity: &EntityRef| {
            let runtime = TeamRuntime::wrap(respawn_inner.clone());
            runtime.run_obelisk_respawn(entity);
        });
        let die_inner = inner.clone();
        let obelisk_die: DieCallback = Rc::new(
            move |entity: &EntityRef,
                  inflictor: &DamageParticipant,
                  attacker: &DamageParticipant,
                  damage: i32,
                  method: i32| {
                let runtime = TeamRuntime::wrap(die_inner.clone());
                runtime.run_obelisk_die(entity, inflictor, attacker, damage, method);
            },
        );
        let touch_inner = inner.clone();
        let obelisk_touch: TouchCallback = Rc::new(
            move |entity: &EntityRef, other: &DamageParticipant, contact: &TouchContact| {
                let runtime = TeamRuntime::wrap(touch_inner.clone());
                runtime.run_obelisk_touch(entity, other, contact);
            },
        );
        let pain_inner = inner.clone();
        let obelisk_pain: PainCallback =
            Rc::new(move |entity: &EntityRef, attacker: &DamageParticipant, amount: f32| {
                let runtime = TeamRuntime::wrap(pain_inner.clone());
                runtime.run_obelisk_pain(entity, attacker, amount);
            });
        let runtime = Self {
            inner,
            obelisk_regen,
            obelisk_respawn,
            obelisk_die,
            obelisk_touch,
            obelisk_pain,
        };
        runtime.bind_save_callbacks();
        runtime
    }

    fn wrap(inner: Rc<TeamInner>) -> Self {
        // Rebuilt only to run a callback body; the stored callbacks below are
        // never read from these transient handles.
        let placeholder_think: ThinkCallback = Rc::new(|_| {});
        let placeholder_die: DieCallback = Rc::new(|_, _, _, _, _| {});
        let placeholder_touch: TouchCallback = Rc::new(|_, _, _| {});
        let placeholder_pain: PainCallback = Rc::new(|_, _, _| {});
        Self {
            inner,
            obelisk_regen: placeholder_think.clone(),
            obelisk_respawn: placeholder_think,
            obelisk_die: placeholder_die,
            obelisk_touch: placeholder_touch,
            obelisk_pain: placeholder_pain,
        }
    }

    /// Checkpoint capture.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveValue {
        let state = self.inner.state.borrow();
        SaveValue::map(vec![
            (
                "state",
                SaveValue::map(vec![
                    ("lastFlagCapture", SaveValue::Int(state.last_flag_capture as i64)),
                    ("lastCaptureTeam", SaveValue::Int(state.last_capture_team as i64)),
                    ("redStatus", SaveValue::Int(state.red_status as i64)),
                    ("blueStatus", SaveValue::Int(state.blue_status as i64)),
                    ("flagStatus", SaveValue::Int(state.flag_status as i64)),
                    ("redTakenTime", SaveValue::Int(state.red_taken_time as i64)),
                    ("blueTakenTime", SaveValue::Int(state.blue_taken_time as i64)),
                    (
                        "redObeliskAttackedTime",
                        SaveValue::Int(state.red_obelisk_attacked_time as i64),
                    ),
                    (
                        "blueObeliskAttackedTime",
                        SaveValue::Int(state.blue_obelisk_attacked_time as i64),
                    ),
                ]),
            ),
            (
                "neutralObelisk",
                match &*self.inner.neutral_obelisk.borrow() {
                    Some(entity) => SaveValue::Int(entity.borrow().slot as i64),
                    None => SaveValue::Null,
                },
            ),
            (
                "lastTeamLocationTime",
                SaveValue::Int(self.inner.last_team_location_time.get() as i64),
            ),
        ])
    }

    /// Checkpoint restore.
    pub fn restore_save_state(&self, value: &SaveValue) {
        let reader = SaveReader::new(value, "q3.team");
        let state = reader.field("state");
        let restored = TeamGameState {
            last_flag_capture: state.field("lastFlagCapture").integer(i64::MIN) as f32,
            last_capture_team: state.field("lastCaptureTeam").integer(i64::MIN) as i32,
            red_status: state.field("redStatus").choice(&[-1, 0, 1, 2, 3, 4]) as i32,
            blue_status: state.field("blueStatus").choice(&[-1, 0, 1, 2, 3, 4]) as i32,
            flag_status: state.field("flagStatus").choice(&[-1, 0, 1, 2, 3, 4]) as i32,
            red_taken_time: state.field("redTakenTime").integer(i64::MIN) as i32,
            blue_taken_time: state.field("blueTakenTime").integer(i64::MIN) as i32,
            red_obelisk_attacked_time: state.field("redObeliskAttackedTime").integer(i64::MIN) as i32,
            blue_obelisk_attacked_time: state.field("blueObeliskAttackedTime").integer(i64::MIN) as i32,
        };
        let pool = self.inner.host.pool();
        let neutral = reader
            .field("neutralObelisk")
            .nullable(|entry| read_module_entity(entry, &pool));
        let time = reader.field("lastTeamLocationTime").integer(i64::MIN);
        *self.inner.state.borrow_mut() = restored;
        *self.inner.neutral_obelisk.borrow_mut() = neutral;
        self.inner.last_team_location_time.set(time as i32);
    }

    /// Reset match state (`initGame`).
    pub fn init_game(&self) {
        *self.inner.state.borrow_mut() = TeamGameState::default();
        if self.inner.host.game_type() == GameType::GtCtf as i32 {
            self.inner.state.borrow_mut().red_status = -1;
            self.inner.state.borrow_mut().blue_status = -1;
            self.set_flag_status(Team::TeamRed as i32, flag_status::AT_BASE);
            self.set_flag_status(Team::TeamBlue as i32, flag_status::AT_BASE);
        } else if self.inner.host.product() == Product::Missionpack
            && self.inner.host.game_type() == GameType::Gt1fctf as i32
        {
            self.inner.state.borrow_mut().flag_status = -1;
            self.set_flag_status(Team::TeamFree as i32, flag_status::AT_BASE);
        }
    }

    /// Print a team message (`printMessage`).
    pub fn print_message(&self, entity: Option<&EntityRef>, text: &str) {
        let end = text.find('\0');
        let message = match end {
            Some(end) => &text[..end],
            None => text,
        }
        .replace('"', "'");
        if message.len() > 1024 {
            panic!("PrintMsg overrun");
        }
        let slot = entity.map(|entity| entity.borrow().slot as i32).unwrap_or(-1);
        self.inner
            .host
            .send_server_command(slot, &format!("print \"{message}\""));
    }

    /// Score for a team with lead-change sounds (`addTeamScore`).
    pub fn add_team_score(&self, origin: Vec3, team_code: i32, score: i32) {
        let event = self
            .inner
            .host
            .pool()
            .temp_entity(origin, EntityEvent::EvGlobalTeamSound as i32);
        event.borrow_mut().r.sv_flags |= ServerEntityFlags::Broadcast as i32;
        let red = self.inner.host.team_scores().get(Team::TeamRed as usize);
        let blue = self.inner.host.team_scores().get(Team::TeamBlue as usize);
        let parm = if team_code == Team::TeamRed as i32 {
            if red.wrapping_add(score) == blue {
                global_team_sound::TIED
            } else if red <= blue && red.wrapping_add(score) > blue {
                global_team_sound::RED_TOOK_LEAD
            } else {
                global_team_sound::RED_SCORED
            }
        } else if blue.wrapping_add(score) == red {
            global_team_sound::TIED
        } else if blue <= red && blue.wrapping_add(score) > red {
            global_team_sound::BLUE_TOOK_LEAD
        } else {
            global_team_sound::BLUE_SCORED
        };
        event.borrow_mut().s.event_parm = parm;
        let current = self.inner.host.team_scores().get(team_code as usize);
        self.inner
            .host
            .team_scores()
            .set(team_code as usize, current.wrapping_add(score));
    }

    /// Publish a flag status (`setFlagStatus`).
    pub fn set_flag_status(&self, team_code: i32, status: i32) {
        let current = if team_code == Team::TeamRed as i32 {
            self.inner.state.borrow().red_status
        } else if team_code == Team::TeamBlue as i32 {
            self.inner.state.borrow().blue_status
        } else if team_code == Team::TeamFree as i32 {
            self.inner.state.borrow().flag_status
        } else {
            return;
        };
        if current == status {
            return;
        }
        {
            let mut state = self.inner.state.borrow_mut();
            if team_code == Team::TeamRed as i32 {
                state.red_status = status;
            } else if team_code == Team::TeamBlue as i32 {
                state.blue_status = status;
            } else {
                state.flag_status = status;
            }
        }
        let value = if self.inner.host.game_type() == GameType::GtCtf as i32 {
            let state = self.inner.state.borrow();
            format!("{}{}", ctf_status(state.red_status), ctf_status(state.blue_status))
        } else {
            format!("{}", self.inner.state.borrow().flag_status)
        };
        let visible = match value.find('\0') {
            Some(nul) => &value[..nul],
            None => &value[..],
        };
        self.inner.host.set_configstring(23, visible);
    }

    /// Mark a dropped flag (`checkDroppedItem`).
    pub fn check_dropped_item(&self, entity: &EntityRef) {
        if let Some(team_code) = flag_team(entity) {
            self.set_flag_status(team_code, flag_status::DROPPED);
        }
    }

    /// Force a team gesture (`forceGesture`).
    pub fn force_gesture(&self, team_code: i32) {
        for index in 0..MAX_CLIENTS {
            let entity = self.inner.host.pool().at(index);
            let body = entity.borrow();
            let same = match &body.client {
                Some(client) => client.borrow().sess.session_team == team_code,
                None => false,
            };
            let live = body.inuse;
            drop(body);
            if live && same {
                entity.borrow_mut().flags |= GameFlags::FORCE_GESTURE;
            }
        }
    }

    fn award(&self, player: &EntityRef, flag: i32) {
        let client = team_client_of(player);
        let mut record = client.borrow_mut();
        record.ps.e_flags = (record.ps.e_flags & !TEAM_AWARDS) | flag;
        record.reward_time = self.inner.host.time().wrapping_add(2000);
    }

    /// Track carrier damage (`checkHurtCarrier`).
    pub fn check_hurt_carrier(&self, target: &EntityRef, attacker: &EntityRef) {
        let (target_client, attacker_client) = (target.borrow().client.clone(), attacker.borrow().client.clone());
        let (Some(target_client), Some(attacker_client)) = (target_client, attacker_client) else {
            return;
        };
        let flag = if target_client.borrow().sess.session_team == Team::TeamRed as i32 {
            Powerup::PwBlueflag as i32
        } else {
            Powerup::PwRedflag as i32
        };
        let target_record = target_client.borrow();
        if (target_record.ps.powerups.get(flag as usize) != 0 || target_record.ps.generic1 != 0)
            && target_record.sess.session_team != attacker_client.borrow().sess.session_team
        {
            attacker_client.borrow_mut().pers.team_state.last_hurt_carrier = self.inner.host.time() as f32;
        }
    }

    /// Frag bonuses (`fragBonuses`).
    pub fn frag_bonuses(&self, target: &EntityRef, attacker: Option<&EntityRef>) {
        let attacker = match attacker {
            Some(attacker) => attacker,
            None => return,
        };
        if target.borrow().client.is_none() || attacker.borrow().client.is_none() {
            return;
        }
        if Rc::ptr_eq(target, attacker) {
            return;
        }
        if on_same_team(self.inner.host.game_type(), target, attacker) {
            return;
        }
        let victim = team_client_of(target);
        let killer = team_client_of(attacker);
        let team_code = victim.borrow().sess.session_team;
        let opposing = other_team(team_code);
        let flag = if team_code == Team::TeamRed as i32 {
            Powerup::PwRedflag as i32
        } else {
            Powerup::PwBlueflag as i32
        };
        let mission = self.inner.host.product() == Product::Missionpack;
        let enemy_flag = if mission && self.inner.host.game_type() == GameType::Gt1fctf as i32 {
            Powerup::PwNeutralflag as i32
        } else if team_code == Team::TeamRed as i32 {
            Powerup::PwBlueflag as i32
        } else {
            Powerup::PwRedflag as i32
        };
        let tokens = if mission && self.inner.host.game_type() == GameType::GtHarvester as i32 {
            victim.borrow().ps.generic1
        } else {
            0
        };
        if victim.borrow().ps.powerups.get(enemy_flag as usize) != 0 || tokens != 0 {
            let has_flag = victim.borrow().ps.powerups.get(enemy_flag as usize) != 0;
            killer.borrow_mut().pers.team_state.last_fragged_carrier = self.inner.host.time() as f32;
            let origin = target.borrow().r.current_origin();
            let base: i32 = if mission { 20 } else { 2 };
            let bonus = if has_flag {
                base
            } else {
                base.wrapping_mul(tokens).wrapping_mul(tokens)
            };
            self.inner.host.add_score(attacker, origin, bonus);
            let frag_carrier = killer.borrow().pers.team_state.frag_carrier;
            killer.borrow_mut().pers.team_state.frag_carrier = frag_carrier.wrapping_add(1);
            let netname = killer.borrow().pers.netname.clone();
            self.print_message(
                None,
                &format!(
                    "{netname}^7 fragged {}'s {} carrier!\n",
                    team_name(team_code),
                    if has_flag { "flag" } else { "skull" }
                ),
            );
            for index in 0..self.inner.host.pool().max_clients() {
                let entity = self.inner.host.pool().at(index);
                if entity.borrow().inuse && team_client_of(&entity).borrow().sess.session_team == opposing {
                    team_client_of(&entity).borrow_mut().pers.team_state.last_hurt_carrier = 0.0;
                }
            }
            return;
        }
        let last_hurt = victim.borrow().pers.team_state.last_hurt_carrier;
        if last_hurt != 0.0 && self.inner.host.time() as f32 - last_hurt < 8000.0 {
            let origin = target.borrow().r.current_origin();
            self.inner.host.add_score(attacker, origin, if mission { 5 } else { 2 });
            let carrier_defense = killer.borrow().pers.team_state.carrier_defense;
            killer.borrow_mut().pers.team_state.carrier_defense = carrier_defense.wrapping_add(1);
            victim.borrow_mut().pers.team_state.last_hurt_carrier = 0.0;
            let record = killer.borrow_mut();
            let defends = record.ps.persistant.get(PersistentIndex::PersDefendCount as usize);
            record
                .ps
                .persistant
                .set(PersistentIndex::PersDefendCount as usize, defends + 1);
            drop(record);
            self.award(attacker, 0x10000);
            return;
        }
        let classname: String;
        let mut carrier: Option<EntityRef> = None;
        if mission && self.inner.host.game_type() == GameType::GtObelisk as i32 {
            let killer_team = killer.borrow().sess.session_team;
            if killer_team != Team::TeamRed as i32 && killer_team != Team::TeamBlue as i32 {
                return;
            }
            classname = if killer_team == Team::TeamRed as i32 {
                "team_redobelisk".to_string()
            } else {
                "team_blueobelisk".to_string()
            };
        } else if mission && self.inner.host.game_type() == GameType::GtHarvester as i32 {
            classname = "team_neutralobelisk".to_string();
        } else {
            let killer_team = killer.borrow().sess.session_team;
            if killer_team != Team::TeamRed as i32 && killer_team != Team::TeamBlue as i32 {
                return;
            }
            classname = if killer_team == Team::TeamRed as i32 {
                "team_CTF_redflag".to_string()
            } else {
                "team_CTF_blueflag".to_string()
            };
            for index in 0..self.inner.host.pool().max_clients() {
                let entity = self.inner.host.pool().at(index);
                if entity.borrow().inuse && team_client_of(&entity).borrow().ps.powerups.get(flag as usize) != 0 {
                    carrier = Some(entity);
                    break;
                }
            }
        }
        let mut base: Option<EntityRef> = None;
        loop {
            base = find_entity(
                &self.inner.host.pool(),
                base.as_ref(),
                EntityStringField::Classname,
                Some(&classname),
            );
            match base.clone() {
                Some(entity) if entity.borrow().flags & GameFlags::DROPPED_ITEM == 0 => {
                    base = Some(entity);
                    break;
                }
                Some(_) => {}
                None => break,
            }
        }
        let base = match base {
            Some(base) => base,
            None => return,
        };
        let base_origin = base.borrow().r.current_origin();
        let target_distance = length3(sub3(target.borrow().r.current_origin(), base_origin));
        let attacker_distance = length3(sub3(attacker.borrow().r.current_origin(), base_origin));
        let target_origin = target.borrow().r.current_origin();
        let attacker_origin = attacker.borrow().r.current_origin();
        if ((target_distance < 1000.0 && self.inner.host.in_pvs(base_origin, target_origin))
            || (attacker_distance < 1000.0 && self.inner.host.in_pvs(base_origin, attacker_origin)))
            && killer.borrow().sess.session_team != victim.borrow().sess.session_team
        {
            self.inner
                .host
                .add_score(attacker, target_origin, if mission { 10 } else { 1 });
            let base_defense = killer.borrow().pers.team_state.base_defense;
            killer.borrow_mut().pers.team_state.base_defense = base_defense.wrapping_add(1);
            let record = killer.borrow_mut();
            let defends = record.ps.persistant.get(PersistentIndex::PersDefendCount as usize);
            record
                .ps
                .persistant
                .set(PersistentIndex::PersDefendCount as usize, defends + 1);
            drop(record);
            self.award(attacker, 0x10000);
            return;
        }
        if let Some(carrier) = carrier {
            if !Rc::ptr_eq(&carrier, attacker) {
                let carrier_origin = carrier.borrow().r.current_origin();
                let distance = length3(sub3(attacker_origin, carrier_origin));
                if ((distance < 1000.0 && self.inner.host.in_pvs(carrier_origin, target_origin))
                    || (attacker_distance < 1000.0 && self.inner.host.in_pvs(carrier_origin, attacker_origin)))
                    && killer.borrow().sess.session_team != victim.borrow().sess.session_team
                {
                    self.inner
                        .host
                        .add_score(attacker, target_origin, if mission { 2 } else { 1 });
                    let carrier_defense = killer.borrow().pers.team_state.carrier_defense;
                    killer.borrow_mut().pers.team_state.carrier_defense = carrier_defense.wrapping_add(1);
                    let record = killer.borrow_mut();
                    let defends = record.ps.persistant.get(PersistentIndex::PersDefendCount as usize);
                    record
                        .ps
                        .persistant
                        .set(PersistentIndex::PersDefendCount as usize, defends + 1);
                    drop(record);
                    self.award(attacker, 0x10000);
                }
            }
        }
    }

    /// Reset one flag (`resetFlag`).
    pub fn reset_flag(&self, team_code: i32) -> Option<EntityRef> {
        let classname = if team_code == Team::TeamRed as i32 {
            "team_CTF_redflag"
        } else if team_code == Team::TeamBlue as i32 {
            "team_CTF_blueflag"
        } else if team_code == Team::TeamFree as i32 {
            "team_CTF_neutralflag"
        } else {
            return None;
        };
        let mut entity: Option<EntityRef> = None;
        let mut base: Option<EntityRef> = None;
        loop {
            entity = find_entity(
                &self.inner.host.pool(),
                entity.as_ref(),
                EntityStringField::Classname,
                Some(classname),
            );
            match entity.clone() {
                Some(found) => {
                    if found.borrow().flags & GameFlags::DROPPED_ITEM != 0 {
                        self.inner.host.pool().free(&found);
                    } else {
                        base = Some(found.clone());
                        self.inner.host.respawn_item(&found);
                    }
                }
                None => break,
            }
        }
        self.set_flag_status(team_code, flag_status::AT_BASE);
        base
    }

    /// Reset the mode's flags (`resetFlags`).
    pub fn reset_flags(&self) {
        if self.inner.host.game_type() == GameType::GtCtf as i32 {
            self.reset_flag(Team::TeamRed as i32);
            self.reset_flag(Team::TeamBlue as i32);
        } else if self.inner.host.product() == Product::Missionpack
            && self.inner.host.game_type() == GameType::Gt1fctf as i32
        {
            self.reset_flag(Team::TeamFree as i32);
        }
    }

    fn flag_sound(&self, entity: &EntityRef, sound: i32) {
        let origin = entity.borrow().s.pos.base;
        let event = self
            .inner
            .host
            .pool()
            .temp_entity(origin, EntityEvent::EvGlobalTeamSound as i32);
        event.borrow_mut().s.event_parm = sound;
        event.borrow_mut().r.sv_flags |= ServerEntityFlags::Broadcast as i32;
    }

    /// Flag-return sound (`returnFlagSound`).
    pub fn return_flag_sound(&self, entity: Option<&EntityRef>, team_code: i32) {
        match entity {
            Some(entity) => self.flag_sound(
                entity,
                if team_code == Team::TeamBlue as i32 {
                    global_team_sound::RED_RETURN
                } else {
                    global_team_sound::BLUE_RETURN
                },
            ),
            None => self.inner.host.warn("Warning:  NULL passed to Team_ReturnFlagSound\n"),
        }
    }

    /// Flag-taken sound (`takeFlagSound`).
    pub fn take_flag_sound(&self, entity: Option<&EntityRef>, team_code: i32) {
        let Some(entity) = entity else {
            self.inner.host.warn("Warning:  NULL passed to Team_TakeFlagSound\n");
            return;
        };
        if team_code == Team::TeamRed as i32 {
            if self.inner.state.borrow().blue_status != flag_status::AT_BASE
                && self.inner.state.borrow().blue_taken_time > self.inner.host.time().wrapping_sub(10_000)
            {
                return;
            }
            self.inner.state.borrow_mut().blue_taken_time = self.inner.host.time();
        } else if team_code == Team::TeamBlue as i32 {
            if self.inner.state.borrow().red_status != flag_status::AT_BASE
                && self.inner.state.borrow().red_taken_time > self.inner.host.time().wrapping_sub(10_000)
            {
                return;
            }
            self.inner.state.borrow_mut().red_taken_time = self.inner.host.time();
        }
        self.flag_sound(
            entity,
            if team_code == Team::TeamBlue as i32 {
                global_team_sound::RED_TAKEN
            } else {
                global_team_sound::BLUE_TAKEN
            },
        );
    }

    /// Flag-capture sound (`captureFlagSound`).
    pub fn capture_flag_sound(&self, entity: Option<&EntityRef>, team_code: i32) {
        match entity {
            Some(entity) => self.flag_sound(
                entity,
                if team_code == Team::TeamBlue as i32 {
                    global_team_sound::BLUE_CAPTURE
                } else {
                    global_team_sound::RED_CAPTURE
                },
            ),
            None => self.inner.host.warn("Warning:  NULL passed to Team_CaptureFlagSound\n"),
        }
    }

    /// Return one flag (`returnFlag`).
    pub fn return_flag(&self, team_code: i32) {
        let base = self.reset_flag(team_code);
        self.return_flag_sound(base.as_ref(), team_code);
        if team_code == Team::TeamFree as i32 {
            self.print_message(None, "The flag has returned!\n");
        } else {
            self.print_message(None, &format!("The {} flag has returned!\n", team_name(team_code)));
        }
    }

    /// Return a freed flag (`freeEntity`).
    pub fn free_entity(&self, entity: &EntityRef) {
        if let Some(team_code) = flag_team(entity) {
            self.return_flag(team_code);
        }
    }

    /// Return a timed-out dropped flag (`droppedFlagThink`).
    pub fn dropped_flag_think(&self, entity: &EntityRef) {
        let team_code = flag_team(entity).unwrap_or(Team::TeamFree as i32);
        let base = self.reset_flag(team_code);
        self.return_flag_sound(base.as_ref(), team_code);
    }

    /// Touch our own flag (`touchOurFlag`).
    pub fn touch_our_flag(&self, entity: &EntityRef, other: &EntityRef, team_code: i32) -> i32 {
        let client = team_client_of(other);
        let mission = self.inner.host.product() == Product::Missionpack;
        let one_flag = mission && self.inner.host.game_type() == GameType::Gt1fctf as i32;
        let session_team = client.borrow().sess.session_team;
        let enemy_flag = if one_flag {
            Powerup::PwNeutralflag as i32
        } else if session_team == Team::TeamRed as i32 {
            Powerup::PwBlueflag as i32
        } else {
            Powerup::PwRedflag as i32
        };
        if !one_flag && entity.borrow().flags & GameFlags::DROPPED_ITEM != 0 {
            let netname = client.borrow().pers.netname.clone();
            self.print_message(
                None,
                &format!("{netname}^7 returned the {} flag!\n", team_name(team_code)),
            );
            let origin = entity.borrow().r.current_origin();
            self.inner.host.add_score(other, origin, if mission { 10 } else { 1 });
            let flag_recovery = client.borrow().pers.team_state.flag_recovery;
            client.borrow_mut().pers.team_state.flag_recovery = flag_recovery.wrapping_add(1);
            client.borrow_mut().pers.team_state.last_returned_flag = self.inner.host.time() as f32;
            let base = self.reset_flag(team_code);
            self.return_flag_sound(base.as_ref(), team_code);
            return 0;
        }
        if client.borrow().ps.powerups.get(enemy_flag as usize) == 0 {
            return 0;
        }
        let netname = client.borrow().pers.netname.clone();
        if one_flag {
            self.print_message(None, &format!("{netname}^7 captured the flag!\n"));
        } else {
            self.print_message(
                None,
                &format!("{netname}^7 captured the {} flag!\n", other_team_name(team_code)),
            );
        }
        client.borrow_mut().ps.powerups.set(enemy_flag as usize, 0);
        self.inner.state.borrow_mut().last_flag_capture = self.inner.host.time() as f32;
        self.inner.state.borrow_mut().last_capture_team = team_code;
        let base_origin = entity.borrow().s.pos.base;
        self.add_team_score(base_origin, session_team, 1);
        self.force_gesture(session_team);
        let captures = client.borrow().pers.team_state.captures;
        client.borrow_mut().pers.team_state.captures = captures.wrapping_add(1);
        self.inner.host.pool().rankings.capture(other.borrow().slot);
        self.award(other, 0x800);
        {
            let record = client.borrow_mut();
            let captures = record.ps.persistant.get(PersistentIndex::PersCaptures as usize);
            record
                .ps
                .persistant
                .set(PersistentIndex::PersCaptures as usize, captures + 1);
        }
        let origin = entity.borrow().r.current_origin();
        self.inner.host.add_score(other, origin, if mission { 100 } else { 5 });
        self.capture_flag_sound(Some(entity), team_code);
        for index in 0..self.inner.host.pool().max_clients() {
            let player = self.inner.host.pool().at(index);
            if !player.borrow().inuse {
                continue;
            }
            let teammate = team_client_of(&player);
            if teammate.borrow().sess.session_team != session_team {
                teammate.borrow_mut().pers.team_state.last_hurt_carrier = -5.0;
            } else {
                if !Rc::ptr_eq(&player, other) {
                    self.inner.host.add_score(&player, origin, if mission { 25 } else { 0 });
                }
                let bonus =
                    if teammate.borrow().pers.team_state.last_returned_flag + 10000.0 > self.inner.host.time() as f32 {
                        Some(if mission { 10 } else { 1 })
                    } else if teammate.borrow().pers.team_state.last_fragged_carrier + 10000.0
                        > self.inner.host.time() as f32
                    {
                        Some(if mission { 10 } else { 2 })
                    } else {
                        None
                    };
                if let Some(bonus) = bonus {
                    self.inner.host.add_score(&player, origin, bonus);
                    let assists = client.borrow().pers.team_state.assists;
                    client.borrow_mut().pers.team_state.assists = assists.wrapping_add(1);
                    let record = teammate.borrow_mut();
                    let assists = record.ps.persistant.get(PersistentIndex::PersAssistCount as usize);
                    record
                        .ps
                        .persistant
                        .set(PersistentIndex::PersAssistCount as usize, assists + 1);
                    drop(record);
                    self.award(&player, 0x20000);
                }
            }
        }
        self.reset_flags();
        self.inner.host.calculate_ranks();
        0
    }

    /// Touch the enemy flag (`touchEnemyFlag`).
    pub fn touch_enemy_flag(&self, entity: &EntityRef, other: &EntityRef, team_code: i32) -> i32 {
        let client = team_client_of(other);
        let mission = self.inner.host.product() == Product::Missionpack;
        let netname = client.borrow().pers.netname.clone();
        if mission && self.inner.host.game_type() == GameType::Gt1fctf as i32 {
            self.print_message(None, &format!("{netname}^7 got the flag!\n"));
            client
                .borrow_mut()
                .ps
                .powerups
                .set(Powerup::PwNeutralflag as usize, 2_147_483_647);
            self.set_flag_status(
                Team::TeamFree as i32,
                if team_code == Team::TeamRed as i32 {
                    flag_status::TAKEN_RED
                } else {
                    flag_status::TAKEN_BLUE
                },
            );
        } else {
            self.print_message(None, &format!("{netname}^7 got the {} flag!\n", team_name(team_code)));
            let flag = if team_code == Team::TeamRed as i32 {
                Powerup::PwRedflag as i32
            } else {
                Powerup::PwBlueflag as i32
            };
            client.borrow_mut().ps.powerups.set(flag as usize, 2_147_483_647);
            self.inner
                .host
                .pool()
                .rankings
                .pickup_powerup(other.borrow().slot, flag);
            self.set_flag_status(team_code, flag_status::TAKEN);
        }
        let origin = entity.borrow().r.current_origin();
        self.inner.host.add_score(other, origin, if mission { 10 } else { 0 });
        client.borrow_mut().pers.team_state.flag_since = self.inner.host.time() as f32;
        self.take_flag_sound(Some(entity), team_code);
        -1
    }

    /// Flag pickup dispatch (`pickupTeam`).
    pub fn pickup_team(&self, entity: &EntityRef, other: &EntityRef) -> i32 {
        let client = team_client_of(other);
        if self.inner.host.product() == Product::Missionpack {
            if self.inner.host.game_type() == GameType::GtObelisk as i32 {
                self.inner.host.pool().free(entity);
                return 0;
            }
            if self.inner.host.game_type() == GameType::GtHarvester as i32 {
                if entity.borrow().spawnflags != client.borrow().sess.session_team {
                    let generic = client.borrow().ps.generic1;
                    client.borrow_mut().ps.generic1 = generic.wrapping_add(1);
                }
                self.inner.host.pool().free(entity);
                return 0;
            }
        }
        let classname = entity.borrow().classname();
        let team_code = if classname.as_deref() == Some("team_CTF_redflag") {
            Some(Team::TeamRed as i32)
        } else if classname.as_deref() == Some("team_CTF_blueflag") {
            Some(Team::TeamBlue as i32)
        } else if self.inner.host.product() == Product::Missionpack
            && classname.as_deref() == Some("team_CTF_neutralflag")
        {
            Some(Team::TeamFree as i32)
        } else {
            None
        };
        let Some(team_code) = team_code else {
            self.print_message(Some(other), "Don't know what team the flag is on.\n");
            return 0;
        };
        if self.inner.host.product() == Product::Missionpack && self.inner.host.game_type() == GameType::Gt1fctf as i32
        {
            if team_code == Team::TeamFree as i32 {
                return self.touch_enemy_flag(entity, other, client.borrow().sess.session_team);
            }
            return if team_code != client.borrow().sess.session_team {
                self.touch_our_flag(entity, other, client.borrow().sess.session_team)
            } else {
                0
            };
        }
        if team_code == client.borrow().sess.session_team {
            self.touch_our_flag(entity, other, team_code)
        } else {
            self.touch_enemy_flag(entity, other, team_code)
        }
    }

    /// Nearest visible location marker (`getLocation`).
    #[must_use]
    pub fn get_location(&self, entity: &EntityRef) -> Option<EntityRef> {
        let mut best: Option<EntityRef> = None;
        let mut best_length = 3.0 * 8192.0 * 8192.0;
        let mut location = self.inner.host.location_head();
        while let Some(marker) = location {
            let delta = sub3(entity.borrow().r.current_origin(), marker.borrow().r.current_origin());
            let length = dot3(delta, delta);
            let origin = entity.borrow().r.current_origin();
            let marker_origin = marker.borrow().r.current_origin();
            if length <= best_length && self.inner.host.in_pvs(origin, marker_origin) {
                best_length = length;
                best = Some(marker.clone());
            }
            location = marker.borrow().next_train.clone();
        }
        best
    }

    /// Location message (`getLocationMessage`).
    #[must_use]
    pub fn get_location_message(&self, entity: &EntityRef, capacity: usize) -> Option<String> {
        let location = self.get_location(entity)?;
        if location.borrow().count != 0 {
            let clamped = location.borrow().count.clamp(0, 7);
            location.borrow_mut().count = clamped;
            let message = location.borrow().message.clone();
            let message_arg = match message {
                Some(text) => GameFormatArgument::Text(text),
                None => GameFormatArgument::Null,
            };
            Some(game_format_bounded(
                "%c%c%s^7",
                &[
                    GameFormatArgument::Int(94),
                    GameFormatArgument::Int(clamped + 48),
                    message_arg,
                ],
                capacity,
            ))
        } else {
            let message = location.borrow().message.clone();
            let message_arg = match message {
                Some(text) => GameFormatArgument::Text(text),
                None => GameFormatArgument::Null,
            };
            Some(game_format_bounded("%s", &[message_arg], capacity))
        }
    }

    /// Team overlay message (`teamplayInfoMessage`).
    pub fn teamplay_info_message(&self, entity: &EntityRef) {
        if !team_client_of(entity).borrow().pers.team_info {
            return;
        }
        let sorted = self.inner.host.sorted_clients();
        let team_code = team_client_of(entity).borrow().sess.session_team;
        let mut clients = Vec::new();
        for (index, client_num) in sorted.iter().enumerate() {
            if index >= self.inner.host.pool().max_clients() || clients.len() >= 32 {
                break;
            }
            let player = self.inner.host.pool().at(*client_num as usize);
            if player.borrow().inuse && team_client_of(&player).borrow().sess.session_team == team_code {
                clients.push(*client_num);
            }
        }
        clients.sort();
        let mut message = String::new();
        let mut count = 0;
        // The source sorts a local clients[] that this output loop never reads.
        for index in 0..self.inner.host.pool().max_clients() {
            if count >= 32 {
                break;
            }
            let player = self.inner.host.pool().at(index);
            if !player.borrow().inuse || team_client_of(&player).borrow().sess.session_team != team_code {
                continue;
            }
            let client = team_client_of(&player);
            let info = client.borrow();
            let armor_slot = match stat_schema(info.ps.product) {
                StatSchema::Base(layout) => layout.armor,
                StatSchema::Missionpack(layout) => layout.armor,
            };
            let entry = game_format_bounded(
                " %i %i %i %i %i %i",
                &[
                    GameFormatArgument::Int(index as i32),
                    GameFormatArgument::Int(info.pers.team_state.location),
                    GameFormatArgument::Int(0.max(info.ps.health())),
                    GameFormatArgument::Int(0.max(info.ps.stats.get(armor_slot as usize))),
                    GameFormatArgument::Int(info.ps.weapon),
                    GameFormatArgument::Int(player.borrow().s.powerups),
                ],
                1024,
            );
            drop(info);
            if message.len() + entry.len() > 8192 {
                break;
            }
            message += &entry;
            count += 1;
        }
        let slot = entity.borrow().slot as i32;
        self.inner.host.send_server_command(
            slot,
            &game_format(
                "tinfo %i %s",
                &[GameFormatArgument::Int(count), GameFormatArgument::Text(message)],
            ),
        );
    }

    /// Periodic team-status refresh (`checkTeamStatus`).
    pub fn check_team_status(&self) {
        if self
            .inner
            .host
            .time()
            .wrapping_sub(self.inner.last_team_location_time.get())
            <= 1000
        {
            return;
        }
        self.inner.last_team_location_time.set(self.inner.host.time());
        for index in 0..self.inner.host.pool().max_clients() {
            let entity = self.inner.host.pool().at(index);
            let client = team_client_of(&entity);
            let record = client.borrow();
            if record.pers.connected != ConnectionState::Connected as i32 {
                continue;
            }
            let inuse = entity.borrow().inuse;
            let team_code = record.sess.session_team;
            drop(record);
            if inuse && (team_code == Team::TeamRed as i32 || team_code == Team::TeamBlue as i32) {
                let location = self
                    .get_location(&entity)
                    .map(|marker| marker.borrow().health)
                    .unwrap_or(0);
                client.borrow_mut().pers.team_state.location = location;
            }
        }
        for index in 0..self.inner.host.pool().max_clients() {
            let entity = self.inner.host.pool().at(index);
            let client = team_client_of(&entity);
            let record = client.borrow();
            if record.pers.connected != ConnectionState::Connected as i32 {
                continue;
            }
            let inuse = entity.borrow().inuse;
            let team_code = record.sess.session_team;
            drop(record);
            if inuse && (team_code == Team::TeamRed as i32 || team_code == Team::TeamBlue as i32) {
                self.teamplay_info_message(&entity);
            }
        }
    }

    fn obelisk_settings(&self) -> ObeliskSettings {
        match self.inner.host.obelisk_settings() {
            Some(settings) => settings,
            None => panic!("Obelisk handlers belong to the missionpack product"),
        }
    }

    fn obelisk_model(&self, entity: &EntityRef) -> EntityRef {
        match entity.borrow().activator.clone() {
            Some(model) => model,
            None => panic!("Obelisk callback requires its spawned model entity"),
        }
    }

    fn run_obelisk_regen(&self, entity: &EntityRef) {
        let settings = self.obelisk_settings();
        entity.borrow_mut().nextthink = self
            .inner
            .host
            .time()
            .wrapping_add(settings.regen_period_seconds.wrapping_mul(1000));
        if entity.borrow().health >= settings.health {
            return;
        }
        self.inner
            .host
            .pool()
            .add_event(entity, EntityEvent::EvPowerupRegen as i32, 0);
        let healed = (entity.borrow().health.wrapping_add(settings.regen_amount)).min(settings.health);
        entity.borrow_mut().health = healed;
        let model = self.obelisk_model(entity);
        model.borrow_mut().s.modelindex2 = obelisk_health_fraction(healed, settings.health);
        model.borrow_mut().s.frame = 0;
    }

    fn run_obelisk_respawn(&self, entity: &EntityRef) {
        let settings = self.obelisk_settings();
        entity.borrow_mut().takedamage = true;
        entity.borrow_mut().health = settings.health;
        let think = self
            .inner
            .host
            .pool()
            .callbacks
            .resolve_think("q3.team-arena.team.obeliskRespawn.think");
        entity.borrow_mut().think = Some(think);
        entity.borrow_mut().nextthink = self
            .inner
            .host
            .time()
            .wrapping_add(settings.regen_period_seconds.wrapping_mul(1000));
        self.obelisk_model(entity).borrow_mut().s.frame = 0;
    }

    fn run_obelisk_die(
        &self,
        entity: &EntityRef,
        _inflictor: &DamageParticipant,
        attacker: &DamageParticipant,
        _damage: i32,
        _method: i32,
    ) {
        let team_code = self.obelisk_team(entity);
        let opposing = other_team(team_code);
        self.add_team_score(entity.borrow().s.pos.base, opposing, 1);
        self.force_gesture(opposing);
        self.inner.host.calculate_ranks();
        entity.borrow_mut().takedamage = false;
        let think = self
            .inner
            .host
            .pool()
            .callbacks
            .resolve_think("q3.team-arena.team.obeliskDie.think");
        entity.borrow_mut().think = Some(think);
        entity.borrow_mut().nextthink = self
            .inner
            .host
            .time()
            .wrapping_add(self.obelisk_settings().respawn_delay_seconds.wrapping_mul(1000));
        let model = self.obelisk_model(entity);
        model.borrow_mut().s.modelindex2 = 255;
        model.borrow_mut().s.frame = 2;
        self.inner
            .host
            .pool()
            .add_event(&model, EntityEvent::EvObeliskexplode as i32, 0);
        if let DamageParticipant::Entity(attacker) = attacker {
            if attacker.borrow().client.is_some() {
                let origin = entity.borrow().r.current_origin();
                self.inner.host.add_score(attacker, origin, 100);
                self.award(attacker, 0x800);
                let client = team_client_of(attacker);
                let record = client.borrow_mut();
                let captures = record.ps.persistant.get(PersistentIndex::PersCaptures as usize);
                record
                    .ps
                    .persistant
                    .set(PersistentIndex::PersCaptures as usize, captures + 1);
            }
        }
        self.inner.state.borrow_mut().red_obelisk_attacked_time = 0;
        self.inner.state.borrow_mut().blue_obelisk_attacked_time = 0;
    }

    fn run_obelisk_touch(&self, entity: &EntityRef, other: &DamageParticipant, _contact: &TouchContact) {
        let DamageParticipant::Entity(other) = other else {
            return;
        };
        let Some(other_client) = other.borrow().client.clone() else {
            return;
        };
        if other_team(other_client.borrow().sess.session_team) != entity.borrow().spawnflags {
            return;
        }
        let tokens = other_client.borrow().ps.generic1;
        if tokens <= 0 {
            return;
        }
        let netname = other_client.borrow().pers.netname.clone();
        self.print_message(
            None,
            &game_format(
                "%s^7 brought in %i skull%s.\n",
                &[
                    GameFormatArgument::Text(netname),
                    GameFormatArgument::Int(tokens),
                    GameFormatArgument::Text(if tokens != 0 { "s".to_string() } else { String::new() }),
                ],
            ),
        );
        let team_code = other_client.borrow().sess.session_team;
        self.add_team_score(entity.borrow().s.pos.base, team_code, tokens);
        self.force_gesture(team_code);
        self.inner
            .host
            .add_score(other, other.borrow().r.current_origin(), 100i32.wrapping_mul(tokens));
        self.award(other, 0x800);
        {
            let mut record = other_client.borrow_mut();
            let captures = record.ps.persistant.get(PersistentIndex::PersCaptures as usize);
            record
                .ps
                .persistant
                .set(PersistentIndex::PersCaptures as usize, captures + tokens);
            record.ps.generic1 = 0;
        }
        self.inner.host.calculate_ranks();
        let obelisk_team = self.obelisk_team(entity);
        self.capture_flag_sound(Some(entity), obelisk_team);
    }

    fn run_obelisk_pain(&self, entity: &EntityRef, attacker: &DamageParticipant, amount: f32) {
        let actual = 1.max((amount / 10.0).trunc() as i32);
        let model = self.obelisk_model(entity);
        model.borrow_mut().s.modelindex2 =
            obelisk_health_fraction(entity.borrow().health, self.obelisk_settings().health);
        if model.borrow().s.frame == 0 {
            self.inner
                .host
                .pool()
                .add_event(entity, EntityEvent::EvObeliskpain as i32, 0);
        }
        model.borrow_mut().s.frame = 1;
        if let DamageParticipant::Entity(attacker) = attacker {
            let origin = entity.borrow().r.current_origin();
            self.inner.host.add_score(attacker, origin, actual);
        }
    }

    fn obelisk_team(&self, entity: &EntityRef) -> i32 {
        match entity.borrow().spawnflags {
            TEAM_FREE | TEAM_RED | TEAM_BLUE | TEAM_SPECTATOR => entity.borrow().spawnflags,
            _ => panic!("Spawned obelisk has no source team"),
        }
    }

    /// Spawn an obelisk trigger (`spawnObelisk`).
    pub fn spawn_obelisk(&self, origin: Vec3, team_code: i32, spawnflags: i32) -> EntityRef {
        let settings = self.obelisk_settings();
        let entity = self.inner.host.pool().spawn();
        entity.borrow_mut().s.origin = origin;
        entity.borrow_mut().s.pos.base = origin;
        entity.borrow_mut().r.set_current_origin(origin);
        entity.borrow_mut().r.mins = vec3(-15.0, -15.0, 0.0);
        entity.borrow_mut().r.maxs = vec3(15.0, 15.0, 87.0);
        entity.borrow_mut().s.e_type = EntityType::EtGeneral as i32;
        entity.borrow_mut().flags = GameFlags::NO_KNOCKBACK;
        if self.inner.host.game_type() == GameType::GtObelisk as i32 {
            entity.borrow_mut().r.contents = 1;
            entity.borrow_mut().takedamage = true;
            entity.borrow_mut().health = settings.health;
            let die = self
                .inner
                .host
                .pool()
                .callbacks
                .resolve_die("q3.team-arena.team.spawnObelisk.die");
            let pain = self
                .inner
                .host
                .pool()
                .callbacks
                .resolve_pain("q3.team-arena.team.spawnObelisk.pain");
            let think = self
                .inner
                .host
                .pool()
                .callbacks
                .resolve_think("q3.team-arena.team.obeliskRespawn.think");
            entity.borrow_mut().die = Some(die);
            entity.borrow_mut().pain = Some(pain);
            entity.borrow_mut().think = Some(think);
            entity.borrow_mut().nextthink = self
                .inner
                .host
                .time()
                .wrapping_add(settings.regen_period_seconds.wrapping_mul(1000));
        }
        if self.inner.host.game_type() == GameType::GtHarvester as i32 {
            entity.borrow_mut().r.contents = 0x40000000;
            let touch = self
                .inner
                .host
                .pool()
                .callbacks
                .resolve_touch("q3.team-arena.team.spawnObelisk.touch");
            entity.borrow_mut().touch = Some(touch);
        }
        if spawnflags & 1 != 0 {
            set_origin(&entity, entity.borrow().s.origin);
        } else {
            entity.borrow_mut().s.origin = vec3(origin.x, origin.y, origin.z + 1.0);
            let start = entity.borrow().s.origin;
            let destination = vec3(start.x, start.y, start.z - 4096.0);
            let (mins, maxs) = {
                let body = entity.borrow();
                (body.r.mins, body.r.maxs)
            };
            let trace = self.inner.host.world().trace_actor(&ActorTraceQuery {
                start,
                end: destination,
                shape: TraceShape::Box { mins, maxs },
                pass_actor: Some(entity.borrow().actor.clone()),
                mask: 1,
            });
            if trace.solidity != TraceSolidity::Clear {
                entity.borrow_mut().s.origin = vec3(start.x, start.y, start.z - 1.0);
                let classname = entity.borrow().classname();
                let origin_text = self.inner.host.pool().vtos(entity.borrow().s.origin);
                let classname_arg = match classname {
                    Some(text) => GameFormatArgument::Text(text),
                    None => GameFormatArgument::Null,
                };
                self.inner.host.warn(&game_format(
                    "SpawnObelisk: %s startsolid at %s\n",
                    &[classname_arg, GameFormatArgument::Text(origin_text)],
                ));
                let pool = self.inner.host.pool();
                write_ground(&entity, None, &pool);
                let settled = entity.borrow().s.origin;
                set_origin(&entity, settled);
            } else {
                let pool = self.inner.host.pool();
                trace_ground(&entity, &trace.hit, &pool);
                set_origin(&entity, trace.end);
            }
        }
        entity.borrow_mut().spawnflags = team_code;
        self.inner.host.world().link(&entity);
        entity
    }

    /// Spawn a team obelisk marker (`spawnTeamObelisk`).
    pub fn spawn_team_obelisk(&self, entity: &EntityRef, team_code: i32) {
        self.obelisk_settings();
        if self.inner.host.game_type() <= GameType::GtTeam as i32 {
            self.inner.host.pool().free(entity);
            return;
        }
        entity.borrow_mut().s.e_type = EntityType::EtTeam as i32;
        if self.inner.host.game_type() == GameType::GtObelisk as i32
            || self.inner.host.game_type() == GameType::GtHarvester as i32
        {
            let origin = entity.borrow().s.origin;
            let spawnflags = entity.borrow().spawnflags;
            let obelisk = self.spawn_obelisk(origin, team_code, spawnflags);
            obelisk.borrow_mut().activator = Some(entity.clone());
            if self.inner.host.game_type() == GameType::GtObelisk as i32 {
                entity.borrow_mut().s.modelindex2 = 255;
                entity.borrow_mut().s.frame = 0;
            }
        }
        entity.borrow_mut().s.modelindex = team_code;
        self.inner.host.world().link(entity);
    }

    /// Spawn a neutral obelisk marker (`spawnNeutralObelisk`).
    pub fn spawn_neutral_obelisk(&self, entity: &EntityRef) {
        self.obelisk_settings();
        if self.inner.host.game_type() != GameType::Gt1fctf as i32
            && self.inner.host.game_type() != GameType::GtHarvester as i32
        {
            self.inner.host.pool().free(entity);
            return;
        }
        entity.borrow_mut().s.e_type = EntityType::EtTeam as i32;
        if self.inner.host.game_type() == GameType::GtHarvester as i32 {
            let origin = entity.borrow().s.origin;
            let spawnflags = entity.borrow().spawnflags;
            let obelisk = self.spawn_obelisk(origin, Team::TeamFree as i32, spawnflags);
            *self.inner.neutral_obelisk.borrow_mut() = Some(obelisk);
        }
        entity.borrow_mut().s.modelindex = Team::TeamFree as i32;
        self.inner.host.world().link(entity);
    }

    /// Announce obelisk attacks (`checkObeliskAttack`).
    pub fn check_obelisk_attack(&self, obelisk: &EntityRef, attacker: &EntityRef) -> bool {
        let is_obelisk = match obelisk.borrow().die.clone() {
            Some(die) => Rc::ptr_eq(&die, &self.obelisk_die),
            None => false,
        };
        if !is_obelisk || attacker.borrow().client.is_none() {
            return false;
        }
        let attacker_team = team_client_of(attacker).borrow().sess.session_team;
        if obelisk.borrow().spawnflags == attacker_team {
            return true;
        }
        let red = obelisk.borrow().spawnflags == Team::TeamRed as i32;
        let blue = obelisk.borrow().spawnflags == Team::TeamBlue as i32;
        let state = self.inner.state.borrow();
        if (red && state.red_obelisk_attacked_time < self.inner.host.time().wrapping_sub(20_000))
            || (blue && state.blue_obelisk_attacked_time < self.inner.host.time().wrapping_sub(20_000))
        {
            drop(state);
            self.flag_sound(
                obelisk,
                if red {
                    global_team_sound::RED_OBELISK_ATTACKED
                } else {
                    global_team_sound::BLUE_OBELISK_ATTACKED
                },
            );
            if red {
                self.inner.state.borrow_mut().red_obelisk_attacked_time = self.inner.host.time();
            } else {
                self.inner.state.borrow_mut().blue_obelisk_attacked_time = self.inner.host.time();
            }
        }
        false
    }

    /// Register save callbacks (`bindSaveCallbacks`).
    pub fn bind_save_callbacks(&self) {
        self.inner
            .host
            .pool()
            .callbacks
            .intern_think("q3.team-arena.team.obeliskRespawn.think", self.obelisk_regen.clone());
        self.inner
            .host
            .pool()
            .callbacks
            .intern_think("q3.team-arena.team.obeliskDie.think", self.obelisk_respawn.clone());
        self.inner
            .host
            .pool()
            .callbacks
            .intern_die("q3.team-arena.team.spawnObelisk.die", self.obelisk_die.clone());
        self.inner
            .host
            .pool()
            .callbacks
            .intern_pain("q3.team-arena.team.spawnObelisk.pain", self.obelisk_pain.clone());
        self.inner
            .host
            .pool()
            .callbacks
            .intern_touch("q3.team-arena.team.spawnObelisk.touch", self.obelisk_touch.clone());
    }
}
