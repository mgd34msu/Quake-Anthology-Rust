//! Native Quake I triggers, buttons, target firing, and toggle lights.
//!
//! Stock `use`/`touch`/`think` gamecode for the trigger family —
//! `trigger_multiple`, `trigger_once`, `trigger_secret`, `trigger_relay`,
//! `trigger_counter`, `trigger_hurt`, `trigger_push`, `trigger_setskill`,
//! `trigger_onlyregistered`, `trigger_teleport` — plus `func_button`,
//! targeted `light` entities, and `info_teleport_destination` records,
//! all driven from [`Q1NativeBehaviors`] through the native
//! touch/think/mover hooks. Monster jump pads wait for monsters;
//! `trigger_changelevel` waits for the app-level map transition; those
//! stay generic inert spawns until then.
//!
//! qsrc: `progs106/triggers.qc` (multi 16-166, relay 179, secret 196,
//! counter 222-270, teleport 279-470, setskill 475-494, onlyregistered
//! 502-536, hurt 538-570, push 572-610), `progs106/buttons.qc` (wait 6,
//! done 16, return 21, blocked 31, fire 36, use 48, touch 54, killed 62,
//! `func_button` 86), `progs106/subs.qc:32` (`InitTrigger`),
//! `progs106/subs.qc:210` (`SUB_UseTargets`), `progs106/misc.qc:19-57`
//! (light use), `WinQuake/pr_edict.c:693` (`ED_NewString` escapes),
//! `WinQuake/common.c:1021` (registered `gfx/pop.lmp` check).
//!
//! Skeleton scope notes: trigger/talk/button/teleport noises have no sim
//! audio path yet (the parsed noise rides the state for the audio
//! slice); centerprints queue in [`Q1NativeBehaviors::centerprints`] for
//! the HUD slice to drain and teleport fogs in
//! [`Q1NativeBehaviors::teleport_fogs`] for the presentation slice;
//! shootable triggers and buttons record health but `th_die` needs
//! damage routing; `trigger_push` skips the grenade branch (no grenades
//! yet); the telefrag invincibility branch waits for powerups.
//!
//! [`Q1NativeBehaviors`]: super::native_q1_spawns::Q1NativeBehaviors

use qa_core::identity::{ActorId, ProviderId};
use qa_core::math::{angle_vectors, vec3, Bounds, Vec3};
use qa_world::body::BodyState;
use qa_world::combat::CombatState;
use qa_world::movers::{use_mover, MoverKind, MoverPhase, MoverState, MoverTable};
use qa_world::server::{Server, ServerLogic};
use qa_world::session::Simulation;
use qa_world::spawn::SpawnFields;
use qa_world::triggers::{TouchContact, TriggerTable};
use qa_world::WorldError;

use super::native_q1_spawns::{
    q1_can_take_damage, q1_door_fire, q1_field_or, q1_health_of, q1_model_index, q1_movedir, q1_remove,
    Q1NativeBehaviors,
};

/// `trigger_multiple` NOTOUCH spawnflag (`triggers.qc:13`): fire only via
/// other entities, never by touching.
const TRIGGER_NOTOUCH: i32 = 1;
/// `trigger_counter` NOMESSAGE spawnflag (`triggers.qc:12`): count down
/// silently.
const TRIGGER_NOMESSAGE: i32 = 1;
/// `trigger_push` PUSH_ONCE spawnflag (`triggers.qc:572`): remove after
/// one push.
const TRIGGER_PUSH_ONCE: i32 = 1;
/// `trigger_teleport` PLAYER_ONLY spawnflag (`triggers.qc:279`).
const TELEPORT_PLAYER_ONLY: i32 = 1;
/// Targeted-light START_OFF spawnflag (`misc.qc:19`): spawn dark.
const LIGHT_START_OFF: i32 = 1;
/// Toggle-light style floor (`misc.qc:49`): only `style >= 32` arms `use`.
const LIGHT_STYLE_MIN: u32 = 32;
/// Stock lightstyle values (`misc.qc:25-30`): `m` is fully on, `a` is off.
const LIGHTSTYLE_ON: char = 'm';
/// Stock lightstyle values (`misc.qc:25-30`): `m` is fully on, `a` is off.
const LIGHTSTYLE_OFF: char = 'a';

/// One entity's `SUB_UseTargets` inputs (`subs.qc:210`): delay first,
/// then message, then killtarget, then target. Empty strings read as
/// unset, matching stock `if (self.field)` on strings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q1UseSource {
    /// Targetname to fire, if any.
    pub target: Option<String>,
    /// Targetname to remove, if any (classic: firing stops here).
    pub killtarget: Option<String>,
    /// Centerprint text with `ED_NewString` escapes resolved, if any.
    pub message: Option<String>,
    /// Fire delay in seconds (0 for none; negative fires at once).
    pub delay: f64,
}

impl Q1UseSource {
    /// Read the firing inputs from spawn fields. Delay parses stock
    /// `atof` (missing or unparseable reads 0, and 0 means no delay —
    /// the zero-selects-default rule does not apply here).
    #[must_use]
    pub fn from_fields(fields: &SpawnFields) -> Self {
        Self {
            target: non_empty(fields.target.clone()),
            killtarget: non_empty(fields.extra.get("killtarget").cloned()),
            message: non_empty(fields.extra.get("message").map(|text| q1_ed_string(text))),
            delay: fields
                .extra
                .get("delay")
                .and_then(|text| text.parse::<f64>().ok())
                .unwrap_or(0.0),
        }
    }

    /// Whether every input is unset (firing is a no-op).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.target.is_none() && self.killtarget.is_none() && self.message.is_none() && self.delay == 0.0
    }
}

/// Keep a string only when it is present and non-empty (stock `if (self.field)`).
fn non_empty(text: Option<String>) -> Option<String> {
    text.filter(|text| !text.is_empty())
}

/// Resolve `ED_NewString` escapes (`pr_edict.c:693-717`): `\n` becomes a
/// newline, any other `\x` becomes one literal backslash (the escaped
/// character is dropped), and a trailing backslash stays literal.
fn q1_ed_string(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 1 < bytes.len() {
            index += 1;
            if bytes[index] == b'n' {
                out.push(b'\n');
            } else {
                out.push(b'\\');
            }
        } else {
            out.push(bytes[index]);
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parsed trigger noise (`trigger_multiple`, `triggers.qc:113-127`):
/// sounds 1/2/3 select the secret/talk/switch cue. The audio slice plays
/// [`Q1TriggerNoise::path`]; until then the selection rides the state so
/// spawn parsing stays testable without an audio path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1TriggerNoise {
    /// `misc/secret.wav`.
    Secret,
    /// `misc/talk.wav`.
    Talk,
    /// `misc/trigger1.wav`.
    Switch,
}

impl Q1TriggerNoise {
    /// Stock sound path for the parsed selection.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        match self {
            Self::Secret => "misc/secret.wav",
            Self::Talk => "misc/talk.wav",
            Self::Switch => "misc/trigger1.wav",
        }
    }

    /// Parse the `sounds` key (1/2/3; anything else arms no noise).
    #[must_use]
    pub fn from_sounds(sounds: i32) -> Option<Self> {
        match sounds {
            1 => Some(Self::Secret),
            2 => Some(Self::Talk),
            3 => Some(Self::Switch),
            _ => None,
        }
    }
}

/// Parsed button noise (`func_button`, `buttons.qc:89-111`): sounds
/// 0/1/2/3 select the steam/clunk/click/in-out cue (four separate `if`s;
/// the default 0 selects steam).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1ButtonNoise {
    /// `buttons/airbut1.wav` (default).
    Air,
    /// `buttons/switch21.wav`.
    Clunk,
    /// `buttons/switch02.wav`.
    Click,
    /// `buttons/switch04.wav`.
    InOut,
}

impl Q1ButtonNoise {
    /// Stock sound path for the parsed selection.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        match self {
            Self::Air => "buttons/airbut1.wav",
            Self::Clunk => "buttons/switch21.wav",
            Self::Click => "buttons/switch02.wav",
            Self::InOut => "buttons/switch04.wav",
        }
    }

    /// Parse the `sounds` key (0/1/2/3; anything else stays silent, like
    /// stock, where no `if` matches and `noise` stays unset).
    #[must_use]
    pub fn from_sounds(sounds: i32) -> Option<Self> {
        match sounds {
            0 => Some(Self::Air),
            1 => Some(Self::Clunk),
            2 => Some(Self::Click),
            3 => Some(Self::InOut),
            _ => None,
        }
    }
}

/// Live trigger gamecode state by kind. `trigger_once` is a multiple
/// with `wait -1` and `trigger_secret` adds the secret count, exactly
/// like stock (`trigger_once`, `triggers.qc:168`; `trigger_secret`,
/// `triggers.qc:196`).
#[derive(Debug, Clone)]
pub enum Q1TriggerKind {
    /// `trigger_multiple`/`trigger_once`/`trigger_secret`.
    Multiple {
        /// Seconds between triggerings (0.2 default; -1 removes).
        wait: f64,
        /// Whether this multiple counts secrets.
        secret: bool,
        /// Shootable health restored by `multi_wait` (0 for touch).
        max_health: f64,
        /// Facing gate from spawn angles (zero vector for any facing).
        movedir: Vec3,
        /// Master-clock seconds until which `multi_trigger` refuses to
        /// refire (the `nextthink > time` guard, `triggers.qc:32`).
        armed_until: f64,
        /// Last activator (`self.enemy`), held for `multi_trigger`.
        enemy: Option<ActorId>,
    },
    /// `trigger_relay`: use-only `SUB_UseTargets` forwarder.
    Relay,
    /// `trigger_counter`: fires after `count` uses.
    Counter {
        /// Remaining uses (2 default).
        count: i32,
        /// Whether the countdown stays silent.
        nomessage: bool,
        /// Last activator (`self.enemy`), held for `multi_trigger`.
        enemy: Option<ActorId>,
    },
    /// `trigger_hurt`: damages takers, then rests 1s as `SOLID_NOT`.
    Hurt {
        /// Damage per touch (5 default).
        dmg: f64,
    },
    /// `trigger_push`: sets velocity along `movedir`.
    Push {
        /// Push direction from spawn angles.
        movedir: Vec3,
        /// Push speed (1000 default; velocity scales by 10).
        speed: f64,
        /// Whether one push removes the trigger.
        once: bool,
    },
    /// `trigger_setskill`: writes the skill value on the start map.
    SetSkill,
    /// `trigger_onlyregistered`: shareware gate with a 2s touch throttle.
    OnlyRegistered {
        /// Master-clock seconds until which touches stay throttled (the
        /// `attack_finished` gate, `triggers.qc:505`).
        attack_until: f64,
    },
    /// `trigger_teleport`: moves the toucher to its destination.
    Teleport {
        /// Whether only the player teleports.
        player_only: bool,
        /// Whether the teleporter has a targetname (fires open a 0.2s
        /// gate instead of always teleporting).
        targeted: bool,
        /// Master-clock instant until which a targeted teleporter stays
        /// open after firing (`nextthink`, `triggers.qc:404`).
        armed_until: f64,
    },
    /// Stock `teledeath` volume (`triggers.qc:343`): telefrags overlaps
    /// for 0.2s, then removes itself.
    Teledeath {
        /// Teleported owner, immune to its own death volume.
        owner: ActorId,
    },
}

/// Live Q1 trigger gamecode state (the QC fields the touch/use/think
/// hooks read).
#[derive(Debug, Clone)]
pub struct Q1Trigger {
    /// Kind-specific state.
    pub kind: Q1TriggerKind,
    /// Firing inputs shared by every firing trigger.
    pub source: Q1UseSource,
    /// Parsed `sounds` selection (audio plays it later).
    pub noise: Option<Q1TriggerNoise>,
}

/// Live `func_button` gamecode state (`buttons.qc`). Travel endpoints,
/// speed, and wait ride the engine [`MoverTable`]; position phases map
/// stock `STATE_BOTTOM`/`STATE_DOWN`/`STATE_UP`/`STATE_TOP`
/// (`defs.qc:322-325`) onto `AtPos1`/`ToPos1`/`ToPos2`/`AtPos2`.
#[derive(Debug, Clone)]
pub struct Q1Button {
    /// Firing inputs (fired on arrival at the pressed position).
    pub source: Q1UseSource,
    /// Last activator (`self.enemy`), held for `button_wait` firing.
    pub enemy: Option<ActorId>,
    /// Parsed `sounds` selection (audio plays it later).
    pub noise: Option<Q1ButtonNoise>,
    /// Shootable health restored by `button_return` (0 for touch).
    pub max_health: f64,
}

/// Live targeted-light gamecode state (`misc.qc:21-57`): style and the
/// on/off bit behind `START_OFF`.
#[derive(Debug, Clone)]
pub struct Q1Light {
    /// Light style number (`use` arms only at 32+).
    pub style: u32,
    /// Whether the light starts (or toggles) off.
    pub start_off: bool,
}

/// One scheduled native think: stock `think`/`nextthink` collapsed onto
/// gamecode-owned state (one slot per actor, like the QC fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1ThinkKind {
    /// `SUB_Remove` (`subs.qc:5`).
    Remove,
    /// `multi_wait` (`triggers.qc:16`): re-arm a fired multiple.
    MultiWait,
    /// `hurt_on` (`triggers.qc:538`): re-solidify a hurt trigger.
    HurtOn,
}

/// One scheduled think with its master-clock due instant.
#[derive(Debug, Clone)]
pub struct Q1PendingThink {
    /// Thinking actor.
    pub actor: ActorId,
    /// Think function.
    pub kind: Q1ThinkKind,
    /// Master-clock seconds at which the think fires.
    pub due_seconds: f64,
}

/// One delayed `SUB_UseTargets` (`subs.qc:217-229`): stock spawns a
/// `DelayedUse` entity, but the payload (message, killtarget, target,
/// enemy-as-activator) needs no actor, so the think pass carries it
/// directly.
#[derive(Debug, Clone)]
pub struct Q1DelayedUse {
    /// Firing inputs copied from the firer.
    pub source: Q1UseSource,
    /// Activator held across the delay (`self.enemy`).
    pub activator: Option<ActorId>,
    /// Master-clock seconds at which the use fires.
    pub due_seconds: f64,
}

/// One queued centerprint for the HUD slice: stock `centerprint`
/// writes the client message buffer immediately, but the walking
/// skeleton has no client path yet, so gamecode queues and the HUD
/// drains. Tests assert the queue instead of pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Centerprint {
    /// Print target (always the player activator).
    pub target: ActorId,
    /// Resolved print text.
    pub text: String,
}

/// Player impulse from gamecode to movement: teleports and pushers move
/// the sim body directly (dedicated servers have no movement state),
/// and the windowed step mirrors the impulse into the authoritative
/// movement state so the next step does not overwrite the body.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PlayerForce {
    /// Forced actor (always the admitted player today; monsters later).
    pub actor: ActorId,
    /// Forced origin, if the impulse moves the actor.
    pub origin: Option<Vec3>,
    /// Forced angles, if the impulse snaps facing.
    pub angles: Option<Vec3>,
    /// Forced velocity, if the impulse sets it.
    pub velocity: Option<Vec3>,
    /// Forced teleport time, if the impulse teleports (players pause
    /// view blending until it lapses).
    pub teleport_time_seconds: Option<f64>,
}

/// Live `info_teleport_destination` record (`triggers.qc:425`): the
/// spawn angles become the arrival facing (`mangle`) and the origin
/// lifts 27 units.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1TeleportDestination {
    /// Arrival origin (spawn origin plus 27 up).
    pub origin: Vec3,
    /// Arrival facing (spawn angles).
    pub mangle: Vec3,
}

/// Register the native Q1 trigger, button, use-only, and inert-change
/// spawn functions. Brush triggers and buttons spawn bodies at the map
/// origin for [`build_q1_trigger`] and [`build_q1_button`] to size;
/// relays, counters, and change triggers spawn bodiless (relay/counter
/// are use-only points, and change triggers wait for the app-level map
/// transition).
pub fn register_q1_trigger_spawns(registry: &mut qa_world::spawn::SpawnRegistry) {
    use qa_world::spawn::SpawnRequest;
    for classname in [
        "trigger_multiple",
        "trigger_once",
        "trigger_secret",
        "trigger_hurt",
        "trigger_push",
        "trigger_setskill",
        "trigger_onlyregistered",
        "trigger_teleport",
        "func_button",
    ] {
        let definition = format!("q1:{classname}");
        registry.register(
            classname,
            Box::new(move |fields| {
                Ok(SpawnRequest {
                    definition: definition.clone(),
                    origin: Some(fields.origin),
                    combat: None,
                    grants: Vec::new(),
                })
            }),
        );
    }
    for classname in [
        "trigger_relay",
        "trigger_counter",
        "trigger_changelevel",
        "info_teleport_destination",
    ] {
        let definition = format!("q1:{classname}");
        registry.register(
            classname,
            Box::new(move |_fields| {
                Ok(SpawnRequest {
                    definition: definition.clone(),
                    origin: None,
                    combat: None,
                    grants: Vec::new(),
                })
            }),
        );
    }
}

/// Whether a classname sizes a brush trigger volume (`InitTrigger`,
/// `subs.qc:32`).
#[must_use]
pub fn q1_is_brush_trigger(classname: &str) -> bool {
    matches!(
        classname,
        "trigger_multiple"
            | "trigger_once"
            | "trigger_secret"
            | "trigger_hurt"
            | "trigger_push"
            | "trigger_setskill"
            | "trigger_onlyregistered"
            | "trigger_teleport"
    )
}

/// Whether a classname spawns bodiless use-only native state.
#[must_use]
pub fn q1_is_use_point(classname: &str) -> bool {
    matches!(classname, "trigger_relay" | "trigger_counter")
}

/// Parse one optional QC integer field: missing or unparseable reads as
/// zero (stock `atoi`).
fn q1_field_int(fields: &SpawnFields, key: &str) -> i32 {
    fields
        .extra
        .get(key)
        .and_then(|text| text.parse::<i32>().ok())
        .unwrap_or(0)
}

/// Parse one raw QC float field: missing or unparseable reads as zero
/// (stock `atof`), with no zero-selects-default rule.
fn q1_field_raw(fields: &SpawnFields, key: &str) -> f64 {
    fields
        .extra
        .get(key)
        .and_then(|text| text.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// Facing/push direction from spawn angles (`InitTrigger`,
/// `subs.qc:32-42`): zero angles keep the zero vector (no restriction),
/// anything else runs `SetMovedir` — including the `(0,-1,0)` up and
/// `(0,-2,0)` down spellings.
fn q1_trigger_movedir(fields: &SpawnFields) -> Vec3 {
    if fields.angles == vec3(0.0, 0.0, 0.0) {
        vec3(0.0, 0.0, 0.0)
    } else {
        q1_movedir(fields.angles)
    }
}

/// Finish a spawned brush-trigger actor: size its body to the brush
/// model (`InitTrigger` `setmodel` sizing), parse its kind, mark its
/// touch volume, and record its gamecode state.
pub fn build_q1_trigger<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    actor: &qa_core::identity::OwnedActor,
    fields: &SpawnFields,
    models: &[Bounds],
) -> Result<(), WorldError> {
    let index = q1_model_index(fields)
        .filter(|index| *index < models.len())
        .ok_or_else(|| {
            WorldError::BadSpawnFields(format!(
                "{} without brush model: {}",
                fields.classname,
                fields.targetname.as_deref().unwrap_or("")
            ))
        })?;
    let bounds = models[index];
    let local = Bounds {
        min: Vec3 {
            x: bounds.min.x - fields.origin.x,
            y: bounds.min.y - fields.origin.y,
            z: bounds.min.z - fields.origin.z,
        },
        max: Vec3 {
            x: bounds.max.x - fields.origin.x,
            y: bounds.max.y - fields.origin.y,
            z: bounds.max.z - fields.origin.z,
        },
    };
    server.simulation_mut().set_body_bounds(actor.id(), local)?;
    let source = Q1UseSource::from_fields(fields);
    let noise = Q1TriggerNoise::from_sounds(q1_field_int(fields, "sounds"));
    match fields.classname.as_str() {
        "trigger_multiple" | "trigger_once" | "trigger_secret" => {
            let secret = fields.classname == "trigger_secret";
            if secret {
                behaviors.total_secrets += 1;
            }
            // `trigger_once` forces `wait -1` before the shared parse
            // (`triggers.qc:168-171`); secrets do the same with default
            // message and sounds (`triggers.qc:196-218`).
            let mut source = source;
            if secret && source.message.is_none() {
                source.message = Some("You found a secret area!".to_string());
            }
            let noise = if secret && noise.is_none() && q1_field_int(fields, "sounds") == 0 {
                Some(Q1TriggerNoise::Secret)
            } else {
                noise
            };
            let wait = if fields.classname == "trigger_multiple" {
                q1_field_or(fields, "wait", 0.2)
            } else {
                -1.0
            };
            let max_health = q1_field_raw(fields, "health");
            let trigger = Q1Trigger {
                kind: Q1TriggerKind::Multiple {
                    wait,
                    secret,
                    max_health,
                    movedir: q1_trigger_movedir(fields),
                    armed_until: 0.0,
                    enemy: None,
                },
                source,
                noise,
            };
            behaviors.triggers.insert(actor.id().clone(), trigger);
            if max_health != 0.0 {
                // Shootable multiples go `SOLID_BBOX` with `takedamage`
                // (`triggers.qc:132-142`); the touch stays null, so the
                // volume never marks. `th_die` needs damage routing.
                server.simulation_mut().set_combat(
                    actor.id(),
                    CombatState {
                        health: max_health,
                        can_take_damage: true,
                        ..CombatState::default()
                    },
                )?;
                behaviors.solids.insert(actor.id().clone());
            } else if fields.spawnflags & TRIGGER_NOTOUCH == 0 {
                server.mark_trigger(actor.id())?;
            }
        }
        "trigger_hurt" => {
            behaviors.triggers.insert(
                actor.id().clone(),
                Q1Trigger {
                    kind: Q1TriggerKind::Hurt {
                        dmg: q1_field_or(fields, "dmg", 5.0),
                    },
                    source,
                    noise: None,
                },
            );
            server.mark_trigger(actor.id())?;
        }
        "trigger_push" => {
            behaviors.triggers.insert(
                actor.id().clone(),
                Q1Trigger {
                    kind: Q1TriggerKind::Push {
                        movedir: q1_trigger_movedir(fields),
                        speed: q1_field_or(fields, "speed", 1000.0),
                        once: fields.spawnflags & TRIGGER_PUSH_ONCE != 0,
                    },
                    source,
                    noise: None,
                },
            );
            server.mark_trigger(actor.id())?;
        }
        "trigger_setskill" => {
            behaviors.triggers.insert(
                actor.id().clone(),
                Q1Trigger {
                    kind: Q1TriggerKind::SetSkill,
                    source,
                    noise: None,
                },
            );
            server.mark_trigger(actor.id())?;
        }
        "trigger_onlyregistered" => {
            behaviors.triggers.insert(
                actor.id().clone(),
                Q1Trigger {
                    kind: Q1TriggerKind::OnlyRegistered { attack_until: 0.0 },
                    source,
                    noise: None,
                },
            );
            server.mark_trigger(actor.id())?;
        }
        "trigger_teleport" => {
            // Stock errors the load without a target
            // (`trigger_teleport`, `triggers.qc:448-470`); the loader
            // skips the record instead of aborting the map.
            if source.target.is_none() {
                return Err(WorldError::BadSpawnFields(
                    "trigger_teleport without target".to_string(),
                ));
            }
            behaviors.triggers.insert(
                actor.id().clone(),
                Q1Trigger {
                    kind: Q1TriggerKind::Teleport {
                        player_only: fields.spawnflags & TELEPORT_PLAYER_ONLY != 0,
                        targeted: fields.targetname.as_deref().is_some_and(|name| !name.is_empty()),
                        armed_until: 0.0,
                    },
                    source,
                    noise: None,
                },
            );
            server.mark_trigger(actor.id())?;
        }
        other => {
            return Err(WorldError::BadSpawnFields(format!("not a brush trigger: {other}")));
        }
    }
    Ok(())
}

/// Parsed `func_button` spawn parameters (`func_button`,
/// `buttons.qc:86-141`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ButtonParams {
    /// Travel speed, units/second (default 40).
    pub speed: f64,
    /// Wait pressed in seconds (default 1; -1 never returns).
    pub wait: f64,
    /// Out position (always the spawn origin).
    pub pos1: Vec3,
    /// Pressed position.
    pub pos2: Vec3,
    /// Shootable health (0 for touch buttons).
    pub health: f64,
}

/// Parse button travel from spawn fields plus the `*N` brush-model
/// bounds in absolute map coords (`buttons.qc:131-140`): `pos2 = pos1 +
/// movedir * (|movedir . size| - lip)`, lip default 4.
pub fn q1_button_params(fields: &SpawnFields, model: &Bounds) -> Q1ButtonParams {
    let size = Vec3 {
        x: model.max.x - model.min.x,
        y: model.max.y - model.min.y,
        z: model.max.z - model.min.z,
    };
    let movedir = q1_movedir(fields.angles);
    let speed = q1_field_or(fields, "speed", 40.0);
    let wait = q1_field_or(fields, "wait", 1.0);
    let lip = q1_field_or(fields, "lip", 4.0);
    let dot = f64::from(movedir.x * size.x + movedir.y * size.y + movedir.z * size.z).abs();
    let travel = dot - lip;
    let pos1 = fields.origin;
    Q1ButtonParams {
        speed,
        wait,
        pos1,
        pos2: Vec3 {
            x: pos1.x + movedir.x * travel as f32,
            y: pos1.y + movedir.y * travel as f32,
            z: pos1.z + movedir.z * travel as f32,
        },
        health: q1_field_raw(fields, "health"),
    }
}

/// Finish a spawned `func_button` actor: size its body to the brush
/// model, register its mover, mark its touch (touch buttons only), and
/// record its gamecode state (`func_button`, `buttons.qc:86-141`).
/// Shootable buttons keep `SOLID_BSP` clip with `takedamage`, like
/// stock; `th_die` needs damage routing.
pub fn build_q1_button<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    actor: &qa_core::identity::OwnedActor,
    fields: &SpawnFields,
    models: &[Bounds],
) -> Result<(), WorldError> {
    let index = q1_model_index(fields)
        .filter(|index| *index < models.len())
        .ok_or_else(|| WorldError::BadSpawnFields(format!("func_button without brush model: {}", fields.classname)))?;
    let model = u32::try_from(index)
        .map_err(|_| WorldError::BadSpawnFields(format!("func_button brush model *{index} out of range")))?;
    let bounds = models[index];
    let local = Bounds {
        min: Vec3 {
            x: bounds.min.x - fields.origin.x,
            y: bounds.min.y - fields.origin.y,
            z: bounds.min.z - fields.origin.z,
        },
        max: Vec3 {
            x: bounds.max.x - fields.origin.x,
            y: bounds.max.y - fields.origin.y,
            z: bounds.max.z - fields.origin.z,
        },
    };
    server.simulation_mut().set_body_bounds(actor.id(), local)?;
    server.simulation_mut().set_body_origin(actor.id(), fields.origin)?;
    let params = q1_button_params(fields, &bounds);
    server.movers_mut().insert(
        actor.id().clone(),
        MoverState::new(MoverKind::Button, params.pos1, params.pos2, params.speed, params.wait),
    );
    behaviors.brush_models.insert(actor.id().clone(), model);
    if params.health != 0.0 {
        server.simulation_mut().set_combat(
            actor.id(),
            CombatState {
                health: params.health,
                can_take_damage: true,
                ..CombatState::default()
            },
        )?;
    } else {
        server.mark_trigger(actor.id())?;
    }
    behaviors.buttons.insert(
        actor.id().clone(),
        Q1Button {
            source: Q1UseSource::from_fields(fields),
            enemy: None,
            noise: Q1ButtonNoise::from_sounds(q1_field_int(fields, "sounds")),
            max_health: params.health,
        },
    );
    Ok(())
}

/// Record a bodiless use point (`trigger_relay`, `trigger_counter`):
/// no volume, no touch, just `use` state. Counters default to 2
/// (`trigger_counter`, `triggers.qc:261-270`).
pub fn q1_note_use_point(behaviors: &mut Q1NativeBehaviors, actor: &ActorId, fields: &SpawnFields) {
    let kind = if fields.classname == "trigger_counter" {
        Q1TriggerKind::Counter {
            count: {
                let count = q1_field_int(fields, "count");
                if count == 0 {
                    2
                } else {
                    count
                }
            },
            nomessage: fields.spawnflags & TRIGGER_NOMESSAGE != 0,
            enemy: None,
        }
    } else {
        Q1TriggerKind::Relay
    };
    behaviors.triggers.insert(
        actor.clone(),
        Q1Trigger {
            kind,
            source: Q1UseSource::from_fields(fields),
            noise: None,
        },
    );
}

/// Record a targeted light with `style >= 32`: `use` toggles it
/// (`misc.qc:41-57`), and spawn writes the initial style (`a` for
/// `START_OFF`, `m` otherwise).
pub fn q1_note_light(behaviors: &mut Q1NativeBehaviors, actor: &ActorId, fields: &SpawnFields) {
    if fields.targetname.as_deref().is_none_or(|name| name.is_empty()) {
        return;
    }
    let style = fields
        .extra
        .get("style")
        .and_then(|text| text.parse::<u32>().ok())
        .unwrap_or(0);
    if style < LIGHT_STYLE_MIN {
        return;
    }
    let start_off = fields.spawnflags & LIGHT_START_OFF != 0;
    behaviors
        .light_styles
        .insert(style, if start_off { LIGHTSTYLE_OFF } else { LIGHTSTYLE_ON });
    behaviors.lights.insert(actor.clone(), Q1Light { style, start_off });
}

/// Record an `info_teleport_destination` (`triggers.qc:425-435`):
/// the spawn angles become the arrival facing, the origin lifts 27
/// units, and a missing targetname fails the record (stock errors the
/// load; the loader skips the record instead).
pub fn q1_note_teleport_destination(
    behaviors: &mut Q1NativeBehaviors,
    actor: &ActorId,
    fields: &SpawnFields,
) -> Result<(), WorldError> {
    if fields.targetname.as_deref().is_none_or(|name| name.is_empty()) {
        return Err(WorldError::BadSpawnFields(
            "info_teleport_destination without targetname".to_string(),
        ));
    }
    behaviors.teleport_destinations.insert(
        actor.clone(),
        Q1TeleportDestination {
            origin: vec3(fields.origin.x, fields.origin.y, fields.origin.z + 27.0),
            mangle: fields.angles,
        },
    );
    Ok(())
}

/// Record the worldspawn `worldtype` (0 medieval, 1 runic, 2 base):
/// key-denial messages name the key per world (`door_touch`,
/// `doors.qc:210-246`).
pub fn q1_note_worldspawn(behaviors: &mut Q1NativeBehaviors, fields: &SpawnFields) {
    behaviors.worldtype = fields
        .extra
        .get("worldtype")
        .and_then(|text| text.parse::<u8>().ok())
        .unwrap_or(0);
}

/// Index a spawned actor under its targetname, in spawn order (stock
/// `find` walks edicts in spawn order; the index preserves it). Every
/// spawned actor indexes — including generic ones — so `killtarget`
/// removal reaches anything with a name.
pub fn q1_note_targetname(behaviors: &mut Q1NativeBehaviors, fields: &SpawnFields, actor: &ActorId) {
    if let Some(name) = fields.targetname.as_deref().filter(|name| !name.is_empty()) {
        behaviors
            .by_targetname
            .entry(name.to_string())
            .or_default()
            .push(actor.clone());
    }
}

/// Whether the Q1 mounts hold the registered version: stock checks for
/// `gfx/pop.lmp` (`COM_CheckRegistered`, `common.c:1021`).
pub fn q1_registered_version(mounts: &qa_content::mounts::MountedContent) -> bool {
    mounts
        .read(qa_content::mounts::ResourceRef::Path("gfx/pop.lmp"))
        .is_ok()
}

/// Fire `SUB_UseTargets` (`subs.qc:210-287`) for one source: delay
/// first (a `DelayedUse` payload, `subs.qc:217-229`), then the message
/// centerprint to a player activator (`subs.qc:235-240`, plus the
/// `misc/talk.wav` the audio slice will play), then killtarget removal
/// (`subs.qc:245-256`), then target firing (`subs.qc:260-284`).
///
/// Stock classic removes every killtarget match and then `return`s
/// without firing targets (`subs.qc:251`); e1m1 relies on this (the
/// `trigger_once` pair `*54`/`*55` kills its message trigger without
/// firing it). Rerelease fires targets anyway
/// (`quakec/subs.qc:266-275`); classic keeps the quirk.
///
/// Stock threads `self`/`other`/`activator` through each fired `use`;
/// no native `use` reads `other`, so only the activator threads through.
pub fn q1_use_targets(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    triggers: &mut TriggerTable,
    source: &Q1UseSource,
    activator: Option<&ActorId>,
) {
    if source.is_empty() {
        return;
    }
    let now = simulation.frame().time.as_seconds_f64();
    if source.delay != 0.0 {
        // Stock copies message/killtarget/target onto the `DelayedUse`
        // entity but never `delay` (`subs.qc:219-228`), so the deferred
        // fire runs at once instead of re-scheduling.
        let mut deferred = source.clone();
        deferred.delay = 0.0;
        behaviors.delayed_uses.push(Q1DelayedUse {
            source: deferred,
            activator: activator.cloned(),
            due_seconds: now + source.delay,
        });
        return;
    }
    if let Some(text) = source.message.as_deref() {
        if activator == behaviors.player.as_ref() {
            if let Some(target) = activator {
                behaviors.centerprints.push(Q1Centerprint {
                    target: (*target).clone(),
                    text: text.to_string(),
                });
            }
        }
    }
    if let Some(kill) = source.killtarget.as_deref() {
        let victims = behaviors.by_targetname.get(kill).cloned().unwrap_or_default();
        for victim in &victims {
            q1_remove(behaviors, simulation, movers, triggers, victim);
        }
        behaviors.by_targetname.remove(kill);
        return;
    }
    if let Some(target) = source.target.as_deref() {
        let fired = behaviors.by_targetname.get(target).cloned().unwrap_or_default();
        for target_actor in &fired {
            q1_fire_use(behaviors, simulation, movers, triggers, target_actor, activator);
        }
    }
}

/// Call one fired entity's `use` function (`subs.qc:268-279`): doors
/// (`door_use`, `doors.qc:146`), buttons (`button_use`,
/// `buttons.qc:48`), multiples (`multi_use`, `triggers.qc:75`),
/// relays (`SUB_UseTargets`), counters (`counter_use`,
/// `triggers.qc:222`), teleports (`teleport_use`, `triggers.qc:436`),
/// and toggle lights (`light_use`, `misc.qc:21`). Anything else is
/// `SUB_Null`, including hurt/push/setskill/gate/teledeath triggers
/// (no `use` function) and teleport destinations.
pub fn q1_fire_use(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    triggers: &mut TriggerTable,
    actor: &ActorId,
    activator: Option<&ActorId>,
) {
    if behaviors.doors.contains_key(actor) {
        // `door_use`: door messages are touch-only, so the target, its
        // master, and the next peer clear theirs before the master fires.
        let master = behaviors.doors.get(actor).map(|door| door.master.clone());
        let Some(master) = master else {
            return;
        };
        let next = behaviors.doors.get(actor).and_then(|door| {
            door.peers
                .iter()
                .position(|peer| peer == actor)
                .and_then(|at| door.peers.get((at + 1) % door.peers.len()).cloned())
        });
        for cleared in [actor.clone(), master.clone()].into_iter().chain(next) {
            if let Some(door) = behaviors.doors.get_mut(&cleared) {
                door.use_source.message = None;
            }
        }
        q1_door_fire(behaviors, simulation, movers, triggers, &master, activator);
        return;
    }
    if behaviors.buttons.contains_key(actor) {
        if let Some(button) = behaviors.buttons.get_mut(actor) {
            button.enemy = activator.cloned();
        }
        q1_button_fire(simulation, movers, actor);
        return;
    }
    if let Some((kind, source)) = behaviors
        .triggers
        .get(actor)
        .map(|trigger| (trigger.kind.clone(), trigger.source.clone()))
    {
        match kind {
            Q1TriggerKind::Multiple { .. } => {
                if let Some(trigger) = behaviors.triggers.get_mut(actor) {
                    if let Q1TriggerKind::Multiple { enemy, .. } = &mut trigger.kind {
                        *enemy = activator.cloned();
                    }
                }
                q1_multi_trigger(behaviors, simulation, movers, triggers, actor);
            }
            Q1TriggerKind::Relay => {
                q1_use_targets(behaviors, simulation, movers, triggers, &source, activator);
            }
            Q1TriggerKind::Counter { .. } => {
                q1_counter_use(behaviors, simulation, movers, triggers, actor, activator);
            }
            Q1TriggerKind::Teleport { .. } => {
                // `teleport_use` (`triggers.qc:436-441`): open a 0.2s
                // gate. `force_retouch` is inherent: the sweep tests
                // every overlap each frame.
                let now = simulation.frame().time.as_seconds_f64();
                if let Some(trigger) = behaviors.triggers.get_mut(actor) {
                    if let Q1TriggerKind::Teleport { armed_until, .. } = &mut trigger.kind {
                        *armed_until = now + 0.2;
                    }
                }
            }
            Q1TriggerKind::Hurt { .. }
            | Q1TriggerKind::Push { .. }
            | Q1TriggerKind::SetSkill
            | Q1TriggerKind::OnlyRegistered { .. }
            | Q1TriggerKind::Teledeath { .. } => {}
        }
        return;
    }
    if let Some(light) = behaviors.lights.get_mut(actor) {
        // `light_use`: below style 32 `use` never arms (spawn
        // guarantees it), so reaching here always toggles.
        if light.start_off {
            behaviors.light_styles.insert(light.style, LIGHTSTYLE_ON);
            light.start_off = false;
        } else {
            behaviors.light_styles.insert(light.style, LIGHTSTYLE_OFF);
            light.start_off = true;
        }
    }
}

/// Run `multi_trigger` (`triggers.qc:30-67`) for a touched or used
/// multiple, or a counter on its final count: the refire guard, the
/// secret count, `takedamage` off, `SUB_UseTargets` with the held
/// enemy, then the wait re-arm or the touch-null plus delayed remove.
fn q1_multi_trigger(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    triggers: &mut TriggerTable,
    actor: &ActorId,
) {
    let now = simulation.frame().time.as_seconds_f64();
    let Some(trigger) = behaviors.triggers.get(actor).cloned() else {
        return;
    };
    let (wait, secret, enemy) = match &trigger.kind {
        Q1TriggerKind::Multiple {
            wait,
            secret,
            armed_until,
            enemy,
            ..
        } => {
            if *armed_until > now {
                return;
            }
            (*wait, *secret, enemy.clone())
        }
        // Counters funnel through `multi_trigger` after the final count
        // (`triggers.qc:255`); their `wait` stays -1, so they remove.
        Q1TriggerKind::Counter { enemy, .. } => (-1.0, false, enemy.clone()),
        _ => return,
    };
    if secret {
        if enemy.as_ref() != behaviors.player.as_ref() {
            return;
        }
        behaviors.found_secrets += 1;
        // Stock broadcasts `SVC_FOUNDSECRET` here; the count is the
        // game state, and the network message rides the net slice.
    }
    // Stock plays `self.noise` here; the audio slice owns playback.
    if let Some(combat) = simulation.combat_state(actor).cloned() {
        let _ignored = simulation.set_combat(
            actor,
            CombatState {
                can_take_damage: false,
                ..combat
            },
        );
    }
    q1_use_targets(behaviors, simulation, movers, triggers, &trigger.source, enemy.as_ref());
    if wait > 0.0 {
        if let Some(trigger) = behaviors.triggers.get_mut(actor) {
            if let Q1TriggerKind::Multiple { armed_until, .. } = &mut trigger.kind {
                *armed_until = now + wait;
            }
        }
        behaviors.schedule_think(actor, Q1ThinkKind::MultiWait, now + wait);
    } else {
        triggers.unmark(actor);
        behaviors.schedule_think(actor, Q1ThinkKind::Remove, now + 0.1);
    }
}

/// Run `counter_use` (`triggers.qc:222-259`): count down, print the
/// remaining steps to a player activator, and `multi_trigger` on the
/// final count.
fn q1_counter_use(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    triggers: &mut TriggerTable,
    actor: &ActorId,
    activator: Option<&ActorId>,
) {
    let Some(trigger) = behaviors.triggers.get(actor).cloned() else {
        return;
    };
    let Q1TriggerKind::Counter { count, nomessage, .. } = &trigger.kind else {
        return;
    };
    let count = *count - 1;
    let nomessage = *nomessage;
    if let Some(trigger) = behaviors.triggers.get_mut(actor) {
        if let Q1TriggerKind::Counter { count: slot, .. } = &mut trigger.kind {
            *slot = count;
        }
    }
    if count < 0 {
        return;
    }
    let to_player = activator == behaviors.player.as_ref();
    if count != 0 {
        if to_player && !nomessage {
            let text = if count >= 4 {
                "There are more to go..."
            } else if count == 3 {
                "Only 3 more to go..."
            } else if count == 2 {
                "Only 2 more to go..."
            } else {
                "Only 1 more to go..."
            };
            if let Some(target) = activator {
                behaviors.centerprints.push(Q1Centerprint {
                    target: (*target).clone(),
                    text: text.to_string(),
                });
            }
        }
        return;
    }
    if to_player && !nomessage {
        if let Some(target) = activator {
            behaviors.centerprints.push(Q1Centerprint {
                target: (*target).clone(),
                text: "Sequence completed!".to_string(),
            });
        }
    }
    if let Some(trigger) = behaviors.triggers.get_mut(actor) {
        if let Q1TriggerKind::Counter { enemy, .. } = &mut trigger.kind {
            *enemy = activator.cloned();
        }
    }
    q1_multi_trigger(behaviors, simulation, movers, triggers, actor);
}

/// Native touch dispatch for Q1 triggers and buttons, chained after
/// the door dispatch: multiples gate on the player plus facing
/// (`multi_touch`, `triggers.qc:81`), hurt checks `takedamage`
/// (`hurt_touch`, `triggers.qc:544`), push checks health
/// (`trigger_push_touch`, `triggers.qc:574`), setskill gates on the
/// player (`trigger_skill_touch`, `triggers.qc:475`), the registered
/// gate throttles 2s (`trigger_onlyregistered_touch`,
/// `triggers.qc:502`), teleports move living solids to their
/// destination (`teleport_touch`, `triggers.qc:368`), teledeaths
/// telefrag non-owners (`tdeath_touch`, `triggers.qc:323`), and buttons
/// gate on the player (`button_touch`, `buttons.qc:54`). Relays and
/// counters are use-only and never marked.
pub fn q1_trigger_touch(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    triggers: &mut TriggerTable,
    contact: &TouchContact,
) {
    if behaviors.buttons.contains_key(&contact.trigger) {
        if Some(&contact.other) != behaviors.player.as_ref() {
            return;
        }
        if let Some(button) = behaviors.buttons.get_mut(&contact.trigger) {
            button.enemy = Some(contact.other.clone());
        }
        q1_button_fire(simulation, movers, &contact.trigger);
        return;
    }
    let Some(kind) = behaviors
        .triggers
        .get(&contact.trigger)
        .map(|trigger| trigger.kind.clone())
    else {
        return;
    };
    match kind {
        Q1TriggerKind::Multiple { movedir, .. } => {
            if Some(&contact.other) != behaviors.player.as_ref() {
                return;
            }
            if movedir != vec3(0.0, 0.0, 0.0) {
                let forward = angle_vectors(behaviors.player_angles).forward;
                if forward.x * movedir.x + forward.y * movedir.y + forward.z * movedir.z < 0.0 {
                    return;
                }
            }
            if let Some(trigger) = behaviors.triggers.get_mut(&contact.trigger) {
                if let Q1TriggerKind::Multiple { enemy, .. } = &mut trigger.kind {
                    *enemy = Some(contact.other.clone());
                }
            }
            q1_multi_trigger(behaviors, simulation, movers, triggers, &contact.trigger);
        }
        Q1TriggerKind::Relay | Q1TriggerKind::Counter { .. } => {}
        Q1TriggerKind::Hurt { dmg } => {
            if !q1_can_take_damage(simulation, &contact.other) {
                return;
            }
            triggers.unmark(&contact.trigger);
            simulation.damage_q1(&contact.other, dmg);
            let now = simulation.frame().time.as_seconds_f64();
            behaviors.schedule_think(&contact.trigger, Q1ThinkKind::HurtOn, now + 1.0);
        }
        Q1TriggerKind::Push { movedir, speed, once } => {
            // Stock pushes grenades first; no grenades exist yet, so the
            // living branch is the whole touch.
            if q1_health_of(simulation, &contact.other) <= 0.0 {
                return;
            }
            let push = (speed * 10.0) as f32;
            let velocity = vec3(movedir.x * push, movedir.y * push, movedir.z * push);
            let _ignored = simulation.set_body_velocity(&contact.other, velocity);
            behaviors.player_forces.push(Q1PlayerForce {
                actor: contact.other.clone(),
                origin: None,
                angles: None,
                velocity: Some(velocity),
                teleport_time_seconds: None,
            });
            // Stock plays `ambience/windfly.wav` for players (throttled
            // 1.5s); the audio slice owns playback.
            if once {
                q1_remove(behaviors, simulation, movers, triggers, &contact.trigger);
            }
        }
        Q1TriggerKind::SetSkill => {
            if Some(&contact.other) != behaviors.player.as_ref() {
                return;
            }
            // Stock `cvar_set ("skill", self.message)`; the value rides
            // the behaviors until the map transition consumes it.
            let value = behaviors
                .triggers
                .get(&contact.trigger)
                .and_then(|trigger| trigger.source.message.clone())
                .unwrap_or_default();
            behaviors.skill_override = Some(value);
        }
        Q1TriggerKind::OnlyRegistered { attack_until } => {
            if Some(&contact.other) != behaviors.player.as_ref() {
                return;
            }
            let now = simulation.frame().time.as_seconds_f64();
            if attack_until > now {
                return;
            }
            if let Some(trigger) = behaviors.triggers.get_mut(&contact.trigger) {
                if let Q1TriggerKind::OnlyRegistered { attack_until } = &mut trigger.kind {
                    *attack_until = now + 2.0;
                }
            }
            if behaviors.registered {
                let mut source = behaviors
                    .triggers
                    .get(&contact.trigger)
                    .map(|trigger| trigger.source.clone())
                    .unwrap_or_default();
                source.message = None;
                let target = contact.trigger.clone();
                q1_use_targets(behaviors, simulation, movers, triggers, &source, Some(&contact.other));
                q1_remove(behaviors, simulation, movers, triggers, &target);
            } else if let Some(text) = behaviors
                .triggers
                .get(&contact.trigger)
                .and_then(|trigger| trigger.source.message.clone())
                .filter(|text| !text.is_empty())
            {
                behaviors.centerprints.push(Q1Centerprint {
                    target: contact.other.clone(),
                    text,
                });
                // Stock plays `misc/talk.wav` here; the audio slice owns it.
            }
        }
        Q1TriggerKind::Teleport { .. } => {
            q1_teleport_touch(
                behaviors,
                simulation,
                movers,
                triggers,
                &contact.trigger,
                &contact.other,
            );
        }
        Q1TriggerKind::Teledeath { owner } => {
            // `tdeath_touch` (`triggers.qc:323`): the owner is immune;
            // anything else with nonzero health takes 50000. The
            // invincible-victim branch (owner explodes itself, frag
            // credit flips) waits for powerups: nothing here carries
            // `invincible_finished` yet.
            if contact.other == owner {
                return;
            }
            if q1_health_of(simulation, &contact.other) != 0.0 {
                simulation.damage_q1(&contact.other, 50_000.0);
            }
        }
    }
}

/// Run `teleport_touch` (`triggers.qc:368-423`): targeted teleporters
/// stay shut until fired, `PLAYER_ONLY` admits just the player, and only
/// living slidebox solids teleport. Firing runs `SUB_UseTargets` with
/// the toucher as activator, queues both fog flashes, spawns the
/// telefrag volume, and moves the toucher; players also snap velocity
/// to 300 along the arrival facing and pause view blending 0.7s.
///
/// Two stock edges degrade instead of crashing the server: a dangling
/// target (stock `objerror`s, `triggers.qc:397`) no-ops, and a toucher
/// the firing killed (stock's `!other.health` path, `triggers.qc:409`)
/// moves its origin only, keeping its velocity — stock's degraded
/// velocity line adds two scaled copies of the same forward vector and
/// drops a component, so there is no coherent value to copy.
fn q1_teleport_touch(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    triggers: &mut TriggerTable,
    actor: &ActorId,
    other: &ActorId,
) {
    let now = simulation.frame().time.as_seconds_f64();
    let Some(trigger) = behaviors.triggers.get(actor).cloned() else {
        return;
    };
    let Q1TriggerKind::Teleport {
        player_only,
        targeted,
        armed_until,
    } = &trigger.kind
    else {
        return;
    };
    // Stock compares `nextthink < time` (`triggers.qc:375`): a targeted
    // teleporter is open at exact gate expiry, shut before ever firing.
    if *targeted && *armed_until < now {
        return;
    }
    let is_player = Some(other) == behaviors.player.as_ref();
    if *player_only && !is_player {
        return;
    }
    if q1_health_of(simulation, other) <= 0.0 || !behaviors.solids.contains(other) {
        return;
    }
    // First targetname match in spawn order, like stock `find` — and it
    // must carry a destination record (a same-named relay is not one).
    let destination = trigger.source.target.as_deref().and_then(|target| {
        behaviors
            .by_targetname
            .get(target)
            .into_iter()
            .flat_map(|matches| matches.iter())
            .find_map(|id| behaviors.teleport_destinations.get(id).cloned())
    });
    let Some(destination) = destination else {
        return;
    };
    q1_use_targets(behaviors, simulation, movers, triggers, &trigger.source, Some(other));
    if let Some(from) = simulation.body_state(other).map(|body| body.origin) {
        behaviors.teleport_fogs.push(from);
    }
    let forward = angle_vectors(destination.mangle).forward;
    behaviors.teleport_fogs.push(vec3(
        destination.origin.x + forward.x * 32.0,
        destination.origin.y + forward.y * 32.0,
        destination.origin.z + forward.z * 32.0,
    ));
    q1_spawn_teledeath(behaviors, simulation, triggers, &destination.origin, other);
    if q1_health_of(simulation, other) <= 0.0 {
        let _ignored = simulation.set_body_origin(other, destination.origin);
        return;
    }
    let _ignored = simulation.set_body_origin(other, destination.origin);
    let _ignored = simulation.set_body_angles(other, destination.mangle);
    // Stock's final flags line parses as `(flags - flags) & FL_ONGROUND`
    // (`triggers.qc:422`), so every stock teleport clears all flags; the
    // sim models only the ground link, cleared here for everyone.
    let _ignored = simulation.clear_body_ground(other);
    if is_player {
        // `fixangle` snaps immediately; the force's angles carry the snap
        // and `teleport_time` pauses view blending (`client.qc`).
        let velocity = vec3(forward.x * 300.0, forward.y * 300.0, forward.z * 300.0);
        let _ignored = simulation.set_body_velocity(other, velocity);
        behaviors.player_forces.push(Q1PlayerForce {
            actor: other.clone(),
            origin: Some(destination.origin),
            angles: Some(destination.mangle),
            velocity: Some(velocity),
            teleport_time_seconds: Some(now + 0.7),
        });
    } else {
        behaviors.player_forces.push(Q1PlayerForce {
            actor: other.clone(),
            origin: Some(destination.origin),
            angles: Some(destination.mangle),
            velocity: None,
            teleport_time_seconds: None,
        });
    }
}

/// Spawn the stock `teledeath` volume (`spawn_tdeath`,
/// `triggers.qc:343-366`): the owner's bounds expanded one unit, at the
/// arrival origin, marked as a trigger, removing itself after 0.2s.
/// `force_retouch` is inherent: the sweep tests every overlap each
/// frame. A full registry skips the volume (stock overflows edicts
/// fatally instead); a bodyless owner sizes from zero bounds.
fn q1_spawn_teledeath(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    triggers: &mut TriggerTable,
    origin: &Vec3,
    owner: &ActorId,
) {
    let now = simulation.frame().time.as_seconds_f64();
    let bounds = simulation.body_state(owner).map_or(
        Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(0.0, 0.0, 0.0),
        },
        |body| body.bounds,
    );
    let spawned = simulation.spawn(
        ProviderId::new("game", "q1"),
        "q1:teledeath",
        Some(BodyState {
            origin: *origin,
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(bounds.min.x - 1.0, bounds.min.y - 1.0, bounds.min.z - 1.0),
                max: vec3(bounds.max.x + 1.0, bounds.max.y + 1.0, bounds.max.z + 1.0),
            },
            ground: None,
        }),
        None,
        Vec::new(),
    );
    let Ok(death) = spawned else {
        return;
    };
    behaviors.triggers.insert(
        death.id().clone(),
        Q1Trigger {
            kind: Q1TriggerKind::Teledeath { owner: owner.clone() },
            source: Q1UseSource::default(),
            noise: None,
        },
    );
    let _ignored = triggers.mark(simulation.registry(), death.id());
    behaviors.schedule_think(death.id(), Q1ThinkKind::Remove, now + 0.2);
}

/// Native think dispatch for Q1 triggers: fire due scheduled thinks
/// (removals, multiple re-arms, hurt re-solidifies) and due delayed
/// uses, each in schedule order. Runs after the mover pass and before
/// the trigger sweep, so removals apply before touches.
pub fn q1_trigger_think(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    triggers: &mut TriggerTable,
) {
    let now = simulation.frame().time.as_seconds_f64();
    let pending = std::mem::take(&mut behaviors.thinks);
    let (due, later): (Vec<Q1PendingThink>, Vec<Q1PendingThink>) =
        pending.into_iter().partition(|think| think.due_seconds <= now);
    behaviors.thinks = later;
    for think in &due {
        match think.kind {
            Q1ThinkKind::Remove => {
                q1_remove(behaviors, simulation, movers, triggers, &think.actor);
            }
            Q1ThinkKind::MultiWait => {
                // `multi_wait` restores shootable multiples; touch
                // multiples re-arm through `armed_until` alone.
                let max_health = behaviors
                    .triggers
                    .get(&think.actor)
                    .and_then(|trigger| match &trigger.kind {
                        Q1TriggerKind::Multiple { max_health, .. } if *max_health != 0.0 => Some(*max_health),
                        _ => None,
                    });
                if let (Some(max_health), Some(combat)) = (max_health, simulation.combat_state(&think.actor).cloned()) {
                    let _ignored = simulation.set_combat(
                        &think.actor,
                        CombatState {
                            health: max_health,
                            can_take_damage: true,
                            ..combat
                        },
                    );
                }
            }
            Q1ThinkKind::HurtOn => {
                let _ignored = triggers.mark(simulation.registry(), &think.actor);
            }
        }
    }
    let delayed = std::mem::take(&mut behaviors.delayed_uses);
    let (due, later): (Vec<Q1DelayedUse>, Vec<Q1DelayedUse>) =
        delayed.into_iter().partition(|pending| pending.due_seconds <= now);
    behaviors.delayed_uses = later;
    for pending in &due {
        q1_use_targets(
            behaviors,
            simulation,
            movers,
            triggers,
            &pending.source,
            pending.activator.as_ref(),
        );
    }
}

/// Fire a button (`button_fire`, `buttons.qc:36`): pressed or pressing
/// buttons ignore the fire, otherwise travel starts toward the pressed
/// position (reversing a return in flight, like stock `SUB_CalcMove`).
/// Targets fire on arrival (`button_wait`), not here.
pub fn q1_button_fire(simulation: &Simulation, movers: &mut MoverTable, actor: &ActorId) {
    let Some(state) = movers.get(actor) else {
        return;
    };
    if matches!(state.phase, MoverPhase::ToPos2 | MoverPhase::AtPos2) {
        return;
    }
    // Stock plays `self.noise` here; the audio slice owns playback.
    let origin = super::native_q1_spawns::q1_mover_origin(simulation, movers, actor);
    if let Some(state) = movers.get_mut(actor) {
        if state.phase == MoverPhase::ToPos1 {
            super::native_q1_spawns::q1_redirect_mover(state, origin, true);
        } else {
            use_mover(state, origin);
        }
    }
}

/// Native mover-think dispatch for Q1 buttons: arrival at the pressed
/// position fires targets (`button_wait`, `buttons.qc:6`; the engine
/// armed the return think for positive waits), the wait think travels
/// back (`button_return`, `buttons.qc:21`; negative waits never arm),
/// mid-travel thinks re-arm the arrival think (the same float-dust
/// resume as doors), and bottom arrival rests (`button_done`,
/// `buttons.qc:16`). Alternate button textures (`frame`) ride the
/// presentation slice.
pub fn q1_button_mover_think(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    triggers: &mut TriggerTable,
    actor: &ActorId,
    phase: MoverPhase,
    arrived: bool,
) {
    let Some(button) = behaviors.buttons.get(actor).cloned() else {
        return;
    };
    match (phase, arrived) {
        (MoverPhase::AtPos2, true) => {
            q1_use_targets(
                behaviors,
                simulation,
                movers,
                triggers,
                &button.source,
                button.enemy.as_ref(),
            );
        }
        (MoverPhase::AtPos2, false) => {
            if button.max_health != 0.0 {
                if let Some(combat) = simulation.combat_state(actor).cloned() {
                    let _ignored = simulation.set_combat(
                        actor,
                        CombatState {
                            can_take_damage: true,
                            ..combat
                        },
                    );
                }
            }
            let origin = super::native_q1_spawns::q1_mover_origin(simulation, movers, actor);
            if let Some(state) = movers.get_mut(actor) {
                use_mover(state, origin);
            }
        }
        (MoverPhase::ToPos1 | MoverPhase::ToPos2, _) => {
            super::native_q1_spawns::q1_rearm_travel(simulation, movers, actor);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_core::math::vec3;
    use qa_core::time::SourceTime;
    use qa_world::body::BodyState;
    use qa_world::combat::CombatState;

    use super::*;
    use crate::options::ApplicationOptions;
    use crate::startup::{open_server, StartupConfig};

    fn test_server() -> Server<qa_guest::server::GuestServerLogic> {
        let config = StartupConfig::from_options(&ApplicationOptions::default()).unwrap();
        open_server(&config).unwrap()
    }

    fn register_all(server: &mut Server<qa_guest::server::GuestServerLogic>) {
        super::super::native_q1_spawns::register_q1_spawns(server.spawns_mut());
        register_q1_trigger_spawns(server.spawns_mut());
    }

    fn trigger_fields(classname: &str, pairs: &[(&str, &str)]) -> SpawnFields {
        let mut full = vec![("classname", classname)];
        full.extend_from_slice(pairs);
        SpawnFields::parse(&full).unwrap()
    }

    fn trigger_model() -> Bounds {
        Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(64.0, 64.0, 64.0),
        }
    }

    fn spawn_brush_trigger(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> qa_core::identity::OwnedActor {
        let actor = server.spawn_entity(fields).unwrap();
        build_q1_trigger(server, behaviors, &actor, fields, &[trigger_model()]).unwrap();
        actor
    }

    fn spawn_use_point(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> qa_core::identity::OwnedActor {
        let actor = server.spawn_entity(fields).unwrap();
        q1_note_use_point(behaviors, actor.id(), fields);
        super::q1_note_targetname(behaviors, fields, actor.id());
        actor
    }

    fn spawn_button(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> qa_core::identity::OwnedActor {
        let actor = server.spawn_entity(fields).unwrap();
        build_q1_button(server, behaviors, &actor, fields, &[trigger_model()]).unwrap();
        super::q1_note_targetname(behaviors, fields, actor.id());
        actor
    }

    fn spawn_player(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        origin: Vec3,
    ) -> qa_core::identity::OwnedActor {
        spawn_player_on(server, origin, None)
    }

    fn spawn_player_on(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        origin: Vec3,
        ground: Option<ActorId>,
    ) -> qa_core::identity::OwnedActor {
        let player = server
            .simulation_mut()
            .spawn(
                qa_core::identity::ProviderId::new("q1", "test"),
                "q1:test_player",
                Some(BodyState {
                    origin,
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: Bounds {
                        min: vec3(-16.0, -16.0, -24.0),
                        max: vec3(16.0, 16.0, 32.0),
                    },
                    ground,
                }),
                None,
                Vec::new(),
            )
            .unwrap();
        server
            .simulation_mut()
            .set_combat(player.id(), CombatState::default())
            .unwrap();
        player
    }

    fn spawn_destination(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> qa_core::identity::OwnedActor {
        let actor = server.spawn_entity(fields).unwrap();
        q1_note_teleport_destination(behaviors, actor.id(), fields).unwrap();
        super::q1_note_targetname(behaviors, fields, actor.id());
        actor
    }

    fn admit_player(behaviors: &mut Q1NativeBehaviors, player: &qa_core::identity::OwnedActor) {
        behaviors.set_player(Some(player.id().clone()));
        behaviors.solids.insert(player.id().clone());
    }

    fn touch(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        trigger: &ActorId,
        other: &ActorId,
    ) {
        let contact = TouchContact {
            trigger: trigger.clone(),
            other: other.clone(),
        };
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_trigger_touch(behaviors, simulation, movers, triggers, &contact);
    }

    fn fire_use(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        actor: &ActorId,
        activator: Option<&ActorId>,
    ) {
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_fire_use(behaviors, simulation, movers, triggers, actor, activator);
    }

    #[test]
    fn use_source_normalizes_empty_strings_and_raw_delay() {
        let fields = trigger_fields(
            "trigger_multiple",
            &[("target", ""), ("killtarget", "k1"), ("message", ""), ("delay", "0")],
        );
        let source = Q1UseSource::from_fields(&fields);
        assert_eq!(source.target, None);
        assert_eq!(source.killtarget.as_deref(), Some("k1"));
        assert_eq!(source.message, None);
        assert_eq!(source.delay, 0.0);
        assert!(!source.is_empty());
        let fields = trigger_fields("trigger_relay", &[("delay", "-1")]);
        assert_eq!(Q1UseSource::from_fields(&fields).delay, -1.0);
        let fields = trigger_fields("trigger_relay", &[]);
        assert!(Q1UseSource::from_fields(&fields).is_empty());
    }

    #[test]
    fn ed_string_resolves_stock_escapes() {
        assert_eq!(q1_ed_string("a\\nb"), "a\nb");
        assert_eq!(q1_ed_string("a\\tb"), "a\\b");
        assert_eq!(q1_ed_string("a\\"), "a\\");
        assert_eq!(q1_ed_string("plain"), "plain");
        let fields = trigger_fields("trigger_multiple", &[("message", "one\\ntwo")]);
        assert_eq!(Q1UseSource::from_fields(&fields).message.as_deref(), Some("one\ntwo"));
    }

    #[test]
    fn multiple_touch_fires_and_rearms_on_wait() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let relay = spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "fired")]),
        );
        assert!(server.simulation().body_state(relay.id()).is_none());
        let multiple = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_multiple", &[("model", "*0"), ("target", "r1"), ("wait", "1")]),
        );
        assert!(server.triggers_mut().is_trigger(multiple.id()));
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        touch(&mut server, &mut behaviors, multiple.id(), player.id());
        assert_eq!(behaviors.centerprints.len(), 1);
        assert_eq!(behaviors.centerprints[0].text, "fired");
        // Still armed: a second touch before the wait lapses refires nothing.
        touch(&mut server, &mut behaviors, multiple.id(), player.id());
        assert_eq!(behaviors.centerprints.len(), 1);
    }

    #[test]
    fn multiple_facing_gate_follows_movedir() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "fired")]),
        );
        let multiple = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "trigger_multiple",
                &[("model", "*0"), ("target", "r1"), ("angle", "270")],
            ),
        );
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        // Facing +Y against a -Y gate: no fire.
        behaviors.player_angles = vec3(0.0, 90.0, 0.0);
        touch(&mut server, &mut behaviors, multiple.id(), player.id());
        assert!(behaviors.centerprints.is_empty());
        // Facing -Y with the gate: fires.
        behaviors.player_angles = vec3(0.0, 270.0, 0.0);
        touch(&mut server, &mut behaviors, multiple.id(), player.id());
        assert_eq!(behaviors.centerprints.len(), 1);
    }

    #[test]
    fn once_removes_after_firing_through_live_ticks() {
        let mut server = test_server();
        register_all(&mut server);
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        spawn_use_point(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "fired")]),
        );
        let multiple = spawn_brush_trigger(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields("trigger_once", &[("model", "*0"), ("target", "r1")]),
        );
        let once = multiple.id().clone();
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        shared.borrow_mut().set_player(Some(player.id().clone()));
        super::super::native_q1_spawns::install_q1_native(&mut server, Rc::clone(&shared));
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        assert_eq!(shared.borrow().centerprints.len(), 1);
        assert!(!server.triggers_mut().is_trigger(&once));
        for _ in 0..4 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert!(simulation_actor_gone(&server, &once));
    }

    fn simulation_actor_gone(server: &Server<qa_guest::server::GuestServerLogic>, actor: &ActorId) -> bool {
        server.simulation().body_state(actor).is_none() && server.simulation().registry().resolve_owned(actor).is_none()
    }

    #[test]
    fn secret_counts_with_default_message_and_noise() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let secret = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_secret", &[("model", "*0")]),
        );
        assert_eq!(behaviors.total_secrets, 1);
        let trigger = behaviors.triggers.get(secret.id()).unwrap();
        assert_eq!(trigger.noise, Some(Q1TriggerNoise::Secret));
        assert_eq!(trigger.source.message.as_deref(), Some("You found a secret area!"));
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        touch(&mut server, &mut behaviors, secret.id(), player.id());
        assert_eq!(behaviors.found_secrets, 1);
        assert_eq!(behaviors.centerprints.len(), 1);
        assert_eq!(behaviors.centerprints[0].text, "You found a secret area!");
    }

    #[test]
    fn notouch_multiple_fires_only_via_use() {
        let mut server = test_server();
        register_all(&mut server);
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        spawn_use_point(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "fired")]),
        );
        let multiple = spawn_brush_trigger(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields(
                "trigger_multiple",
                &[
                    ("model", "*0"),
                    ("target", "r1"),
                    ("targetname", "m1"),
                    ("spawnflags", "1"),
                ],
            ),
        );
        assert!(!server.triggers_mut().is_trigger(multiple.id()));
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        shared.borrow_mut().set_player(Some(player.id().clone()));
        super::super::native_q1_spawns::install_q1_native(&mut server, Rc::clone(&shared));
        // Standing inside the unmarked volume touches nothing.
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        assert!(shared.borrow().centerprints.is_empty());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_fire_use(
            &mut shared.borrow_mut(),
            simulation,
            movers,
            triggers,
            multiple.id(),
            Some(player.id()),
        );
        assert_eq!(shared.borrow().centerprints.len(), 1);
    }

    #[test]
    fn shootable_multiple_arms_bbox_without_touch() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "fired")]),
        );
        let multiple = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "trigger_multiple",
                &[("model", "*0"), ("target", "r1"), ("health", "10"), ("wait", "1")],
            ),
        );
        assert!(!server.triggers_mut().is_trigger(multiple.id()));
        assert!(behaviors.solids.contains(multiple.id()));
        assert_eq!(
            server
                .simulation()
                .combat_state(multiple.id())
                .map(|combat| combat.health),
            Some(10.0)
        );
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        fire_use(&mut server, &mut behaviors, multiple.id(), Some(player.id()));
        assert_eq!(behaviors.centerprints.len(), 1);
        assert!(!server.simulation().combat_state(multiple.id()).unwrap().can_take_damage);
    }

    #[test]
    fn counter_counts_down_then_fires_and_removes() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "open")]),
        );
        let counter = spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "trigger_counter",
                &[("targetname", "c1"), ("target", "r1"), ("count", "3")],
            ),
        );
        let player = spawn_player(&mut server, vec3(0.0, 0.0, 200.0));
        behaviors.set_player(Some(player.id().clone()));
        fire_use(&mut server, &mut behaviors, counter.id(), Some(player.id()));
        assert_eq!(behaviors.centerprints.len(), 1);
        assert_eq!(behaviors.centerprints[0].text, "Only 2 more to go...");
        fire_use(&mut server, &mut behaviors, counter.id(), Some(player.id()));
        assert_eq!(behaviors.centerprints[1].text, "Only 1 more to go...");
        fire_use(&mut server, &mut behaviors, counter.id(), Some(player.id()));
        assert_eq!(behaviors.centerprints[2].text, "Sequence completed!");
        assert_eq!(behaviors.centerprints[3].text, "open");
        // Spent: further uses stay silent.
        fire_use(&mut server, &mut behaviors, counter.id(), Some(player.id()));
        assert_eq!(behaviors.centerprints.len(), 4);
    }

    #[test]
    fn killtarget_removes_without_firing_targets() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        // e1m1 `*54` shape: a once that kills and targets the same name.
        let victim = spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "t31"), ("message", "must not print")]),
        );
        let killer = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "trigger_once",
                &[("model", "*0"), ("target", "t31"), ("killtarget", "t31")],
            ),
        );
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        touch(&mut server, &mut behaviors, killer.id(), player.id());
        assert!(simulation_actor_gone(&server, victim.id()));
        assert!(behaviors.centerprints.is_empty());
    }

    #[test]
    fn delay_defers_firing_until_due() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "late")]),
        );
        let relay = spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "trigger_relay",
                &[("targetname", "d1"), ("target", "r1"), ("delay", "1")],
            ),
        );
        let player = spawn_player(&mut server, vec3(0.0, 0.0, 200.0));
        behaviors.set_player(Some(player.id().clone()));
        fire_use(&mut server, &mut behaviors, relay.id(), Some(player.id()));
        assert!(behaviors.centerprints.is_empty());
        assert_eq!(behaviors.delayed_uses.len(), 1);
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        {
            let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
            q1_trigger_think(&mut behaviors, simulation, movers, triggers);
        }
        assert!(behaviors.centerprints.is_empty());
        for _ in 0..25 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        {
            let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
            q1_trigger_think(&mut behaviors, simulation, movers, triggers);
        }
        assert_eq!(behaviors.centerprints.len(), 1);
        assert_eq!(behaviors.centerprints[0].text, "late");
    }

    #[test]
    fn message_prints_only_to_player_activators() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let relay = spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "hi")]),
        );
        let player = spawn_player(&mut server, vec3(0.0, 0.0, 200.0));
        let other = spawn_player(&mut server, vec3(0.0, 0.0, 300.0));
        behaviors.set_player(Some(player.id().clone()));
        fire_use(&mut server, &mut behaviors, relay.id(), Some(other.id()));
        fire_use(&mut server, &mut behaviors, relay.id(), None);
        assert!(behaviors.centerprints.is_empty());
        fire_use(&mut server, &mut behaviors, relay.id(), Some(player.id()));
        assert_eq!(behaviors.centerprints.len(), 1);
    }

    #[test]
    fn hurt_damages_takers_and_rearms_through_live_ticks() {
        let mut server = test_server();
        register_all(&mut server);
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        let hurt = spawn_brush_trigger(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields("trigger_hurt", &[("model", "*0"), ("dmg", "7")]),
        );
        let hurt_id = hurt.id().clone();
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        shared.borrow_mut().set_player(Some(player.id().clone()));
        super::super::native_q1_spawns::install_q1_native(&mut server, Rc::clone(&shared));
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        assert_eq!(
            server
                .simulation()
                .combat_state(player.id())
                .map(|combat| combat.health),
            Some(93.0)
        );
        assert!(!server.triggers_mut().is_trigger(&hurt_id));
        // Step clear so the re-armed volume does not bite again, then
        // run out the 1s rest: the volume re-marks with no more damage.
        server
            .simulation_mut()
            .set_body_origin(player.id(), vec3(32.0, 32.0, 500.0))
            .unwrap();
        for _ in 0..25 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert!(server.triggers_mut().is_trigger(&hurt_id));
        assert_eq!(
            server
                .simulation()
                .combat_state(player.id())
                .map(|combat| combat.health),
            Some(93.0)
        );
    }

    #[test]
    fn hurt_ignores_entities_without_takedamage() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let hurt = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_hurt", &[("model", "*0")]),
        );
        // A body with no combat takes no damage and the volume stays armed.
        let inert = server
            .simulation_mut()
            .spawn(
                qa_core::identity::ProviderId::new("q1", "test"),
                "q1:inert",
                Some(BodyState {
                    origin: vec3(32.0, 32.0, 32.0),
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: Bounds {
                        min: vec3(-8.0, -8.0, -8.0),
                        max: vec3(8.0, 8.0, 8.0),
                    },
                    ground: None,
                }),
                None,
                Vec::new(),
            )
            .unwrap();
        touch(&mut server, &mut behaviors, hurt.id(), inert.id());
        assert!(server.triggers_mut().is_trigger(hurt.id()));
        assert!(server.simulation().combat_state(inert.id()).is_none());
    }

    #[test]
    fn push_sets_velocity_and_queues_the_movement_force() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let push = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_push", &[("model", "*0"), ("angle", "90"), ("speed", "100")]),
        );
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        touch(&mut server, &mut behaviors, push.id(), player.id());
        let body = server.simulation().body_state(player.id()).unwrap();
        assert!((f64::from(body.velocity.x)).abs() < 1e-3);
        assert!((f64::from(body.velocity.y) - 1000.0).abs() < 1e-3);
        assert_eq!(behaviors.player_forces.len(), 1);
        assert_eq!(behaviors.player_forces[0].velocity, Some(body.velocity));
        assert!(server.triggers_mut().is_trigger(push.id()));
    }

    #[test]
    fn push_once_removes_and_dead_stays_put() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        // Angle 360 pushes +X (angle 0 would keep the zero vector, per
        // `InitTrigger`).
        let push = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "trigger_push",
                &[("model", "*0"), ("angle", "360"), ("spawnflags", "1")],
            ),
        );
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        server.simulation_mut().damage_q1(player.id(), 1000.0);
        touch(&mut server, &mut behaviors, push.id(), player.id());
        let body = server.simulation().body_state(player.id()).unwrap();
        assert_eq!(body.velocity, vec3(0.0, 0.0, 0.0));
        assert!(!simulation_actor_gone(&server, push.id()));
        // Revived, the touch pushes and the once branch removes.
        server
            .simulation_mut()
            .set_combat(player.id(), CombatState::default())
            .unwrap();
        touch(&mut server, &mut behaviors, push.id(), player.id());
        let body = server.simulation().body_state(player.id()).unwrap();
        assert!((f64::from(body.velocity.x) - 10_000.0).abs() < 1e-2);
        assert!(simulation_actor_gone(&server, push.id()));
    }

    #[test]
    fn setskill_records_the_skill_value_for_the_player_only() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let skill = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_setskill", &[("model", "*0"), ("message", "2")]),
        );
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        let other = spawn_player(&mut server, vec3(40.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        touch(&mut server, &mut behaviors, skill.id(), other.id());
        assert_eq!(behaviors.skill_override, None);
        touch(&mut server, &mut behaviors, skill.id(), player.id());
        assert_eq!(behaviors.skill_override.as_deref(), Some("2"));
    }

    #[test]
    fn onlyregistered_prints_on_shareware_and_fires_when_registered() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "registered path")]),
        );
        let gate = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "trigger_onlyregistered",
                &[("model", "*0"), ("target", "r1"), ("message", "register!")],
            ),
        );
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        behaviors.registered = false;
        touch(&mut server, &mut behaviors, gate.id(), player.id());
        assert_eq!(behaviors.centerprints.len(), 1);
        assert_eq!(behaviors.centerprints[0].text, "register!");
        assert!(!simulation_actor_gone(&server, gate.id()));
        // Throttled: the shareware touch refires nothing for 2s.
        touch(&mut server, &mut behaviors, gate.id(), player.id());
        assert_eq!(behaviors.centerprints.len(), 1);
        behaviors.registered = true;
        // A fresh gate fires its targets and removes itself instead.
        let gate = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "trigger_onlyregistered",
                &[("model", "*0"), ("target", "r1"), ("message", "register!")],
            ),
        );
        touch(&mut server, &mut behaviors, gate.id(), player.id());
        assert_eq!(behaviors.centerprints.len(), 2);
        assert_eq!(behaviors.centerprints[1].text, "registered path");
        assert!(simulation_actor_gone(&server, gate.id()));
    }

    #[test]
    fn button_params_apply_stock_defaults_and_travel() {
        let fields = trigger_fields("func_button", &[("angle", "90"), ("origin", "0 0 0"), ("model", "*0")]);
        let params = q1_button_params(&fields, &trigger_model());
        assert_eq!(params.speed, 40.0);
        assert_eq!(params.wait, 1.0);
        assert_eq!(params.health, 0.0);
        // |movedir . size| - lip = 64 - 4 along +Y.
        assert!((f64::from(params.pos2.x)).abs() < 1e-4);
        assert!((f64::from(params.pos2.y) - 60.0).abs() < 1e-4);
        let fields = trigger_fields(
            "func_button",
            &[("speed", "0"), ("wait", "0"), ("lip", "0"), ("model", "*0")],
        );
        let params = q1_button_params(&fields, &trigger_model());
        assert_eq!((params.speed, params.wait), (40.0, 1.0));
    }

    #[test]
    fn button_touch_presses_fires_returns_through_live_ticks() {
        let mut server = test_server();
        register_all(&mut server);
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        spawn_use_point(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "pressed")]),
        );
        let button = spawn_button(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields(
                "func_button",
                &[
                    ("model", "*0"),
                    ("angle", "90"),
                    ("speed", "4000"),
                    ("wait", "0.2"),
                    ("target", "r1"),
                ],
            ),
        );
        let button_id = button.id().clone();
        assert!(server.triggers_mut().is_trigger(&button_id));
        assert!(shared.borrow().brush_models.contains_key(&button_id));
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        shared.borrow_mut().set_player(Some(player.id().clone()));
        super::super::native_q1_spawns::install_q1_native(&mut server, Rc::clone(&shared));
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        assert_eq!(
            server.movers_mut().get(&button_id).map(|mover| mover.phase),
            Some(MoverPhase::ToPos2)
        );
        // Step clear so the travelling brush never blocks on the player,
        // then run out the press, the target fire, and the return.
        server
            .simulation_mut()
            .set_body_origin(player.id(), vec3(32.0, 32.0, 500.0))
            .unwrap();
        for _ in 0..4 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert_eq!(shared.borrow().centerprints.len(), 1);
        assert_eq!(shared.borrow().centerprints[0].text, "pressed");
        for _ in 0..10 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert_eq!(
            server.movers_mut().get(&button_id).map(|mover| mover.phase),
            Some(MoverPhase::AtPos1)
        );
    }

    #[test]
    fn button_ignores_fire_while_pressing_or_pressed() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let button = spawn_button(
            &mut server,
            &mut behaviors,
            &trigger_fields("func_button", &[("model", "*0"), ("angle", "90"), ("speed", "40")]),
        );
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_button_fire(simulation, movers, button.id());
        }
        let armed = server
            .movers_mut()
            .get(button.id())
            .map(|mover| mover.next_think_seconds);
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_button_fire(simulation, movers, button.id());
        }
        assert_eq!(
            server.movers_mut().get(button.id()).map(|mover| mover.phase),
            Some(MoverPhase::ToPos2)
        );
        assert_eq!(
            server
                .movers_mut()
                .get(button.id())
                .map(|mover| mover.next_think_seconds),
            armed
        );
        server.movers_mut().get_mut(button.id()).unwrap().phase = MoverPhase::AtPos2;
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_button_fire(simulation, movers, button.id());
        }
        assert_eq!(
            server.movers_mut().get(button.id()).map(|mover| mover.phase),
            Some(MoverPhase::AtPos2)
        );
    }

    #[test]
    fn button_wait_negative_stays_pressed_through_live_ticks() {
        let mut server = test_server();
        register_all(&mut server);
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        let button = spawn_button(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields(
                "func_button",
                &[("model", "*0"), ("angle", "90"), ("speed", "4000"), ("wait", "-1")],
            ),
        );
        let button_id = button.id().clone();
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        shared.borrow_mut().set_player(Some(player.id().clone()));
        super::super::native_q1_spawns::install_q1_native(&mut server, Rc::clone(&shared));
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        server
            .simulation_mut()
            .set_body_origin(player.id(), vec3(32.0, 32.0, 500.0))
            .unwrap();
        for _ in 0..30 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert_eq!(
            server.movers_mut().get(&button_id).map(|mover| mover.phase),
            Some(MoverPhase::AtPos2)
        );
    }

    #[test]
    fn button_blocked_is_a_noop() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let button = spawn_button(
            &mut server,
            &mut behaviors,
            &trigger_fields("func_button", &[("model", "*0"), ("angle", "90")]),
        );
        server.movers_mut().get_mut(button.id()).unwrap().phase = MoverPhase::ToPos1;
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        super::super::native_q1_spawns::q1_native_mover_blocked(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            button.id(),
            player.id(),
        );
        assert_eq!(
            server.movers_mut().get(button.id()).map(|mover| mover.phase),
            Some(MoverPhase::ToPos1)
        );
        assert_eq!(
            server
                .simulation()
                .combat_state(player.id())
                .map(|combat| combat.health),
            Some(100.0)
        );
    }

    #[test]
    fn shootable_button_arms_takedamage_without_touch() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let button = spawn_button(
            &mut server,
            &mut behaviors,
            &trigger_fields("func_button", &[("model", "*0"), ("angle", "90"), ("health", "20")]),
        );
        assert!(!server.triggers_mut().is_trigger(button.id()));
        assert!(shared_brush_clip(&behaviors, button.id()));
        assert_eq!(
            server
                .simulation()
                .combat_state(button.id())
                .map(|combat| combat.health),
            Some(20.0)
        );
    }

    fn shared_brush_clip(behaviors: &Q1NativeBehaviors, actor: &ActorId) -> bool {
        behaviors.brush_models.contains_key(actor)
    }

    #[test]
    fn lights_arm_only_at_style_32_and_toggle() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let fields = trigger_fields("light", &[("targetname", "l1"), ("style", "33"), ("spawnflags", "1")]);
        let actor = server.spawn_entity(&fields).unwrap();
        q1_note_light(&mut behaviors, actor.id(), &fields);
        assert_eq!(behaviors.light_styles.get(&33), Some(&'a'));
        fire_use(&mut server, &mut behaviors, actor.id(), None);
        assert_eq!(behaviors.light_styles.get(&33), Some(&'m'));
        fire_use(&mut server, &mut behaviors, actor.id(), None);
        assert_eq!(behaviors.light_styles.get(&33), Some(&'a'));
        let fields = trigger_fields("light", &[("targetname", "l2"), ("style", "16")]);
        let actor = server.spawn_entity(&fields).unwrap();
        q1_note_light(&mut behaviors, actor.id(), &fields);
        assert!(!behaviors.lights.contains_key(actor.id()));
    }

    #[test]
    fn door_fire_clears_messages_and_travel_fires_targets() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "r1"), ("message", "door opened")]),
        );
        let fields = trigger_fields(
            "func_door",
            &[
                ("model", "*0"),
                ("targetname", "d1"),
                ("target", "r1"),
                ("message", "touch me"),
            ],
        );
        let actor = server.spawn_entity(&fields).unwrap();
        super::super::native_q1_spawns::build_q1_door(&mut server, &mut behaviors, &actor, &fields, &[trigger_model()])
            .unwrap();
        super::q1_note_targetname(&mut behaviors, &fields, actor.id());
        let player = spawn_player(&mut server, vec3(200.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        fire_use(&mut server, &mut behaviors, actor.id(), Some(player.id()));
        assert_eq!(
            server.movers_mut().get(actor.id()).map(|mover| mover.phase),
            Some(MoverPhase::ToPos2)
        );
        assert_eq!(behaviors.centerprints.len(), 1);
        assert_eq!(behaviors.centerprints[0].text, "door opened");
        assert_eq!(
            behaviors
                .doors
                .get(actor.id())
                .and_then(|door| door.use_source.message.clone()),
            None
        );
    }

    #[test]
    fn door_touch_prints_message_and_key_denial_names_the_key() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        behaviors.worldtype = 2;
        let fields = trigger_fields(
            "func_door",
            &[("model", "*0"), ("spawnflags", "16"), ("message", "locked")],
        );
        let actor = server.spawn_entity(&fields).unwrap();
        super::super::native_q1_spawns::build_q1_door(&mut server, &mut behaviors, &actor, &fields, &[trigger_model()])
            .unwrap();
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        let contact = TouchContact {
            trigger: actor.id().clone(),
            other: player.id().clone(),
        };
        {
            let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
            super::super::native_q1_spawns::q1_native_touch(&mut behaviors, simulation, movers, triggers, &contact);
        }
        assert_eq!(behaviors.centerprints.len(), 2);
        assert_eq!(behaviors.centerprints[0].text, "locked");
        assert_eq!(behaviors.centerprints[1].text, "You need the silver keycard");
        assert_eq!(
            server.movers_mut().get(actor.id()).map(|mover| mover.phase),
            Some(MoverPhase::AtPos1)
        );
    }

    #[test]
    fn targetname_index_fires_in_spawn_order() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "r"), ("message", "first")]),
        );
        spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "r"), ("message", "second")]),
        );
        let player = spawn_player(&mut server, vec3(0.0, 0.0, 200.0));
        behaviors.set_player(Some(player.id().clone()));
        let source = Q1UseSource {
            target: Some("r".to_string()),
            ..Q1UseSource::default()
        };
        {
            let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
            q1_use_targets(&mut behaviors, simulation, movers, triggers, &source, Some(player.id()));
        }
        let texts: Vec<&str> = behaviors.centerprints.iter().map(|print| print.text.as_str()).collect();
        assert_eq!(texts, ["first", "second"]);
    }

    #[test]
    fn thinks_replace_per_actor() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let hurt = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_hurt", &[("model", "*0")]),
        );
        behaviors.schedule_think(hurt.id(), Q1ThinkKind::HurtOn, 1.0);
        behaviors.schedule_think(hurt.id(), Q1ThinkKind::Remove, 2.0);
        assert_eq!(behaviors.thinks.len(), 1);
        assert_eq!(behaviors.thinks[0].kind, Q1ThinkKind::Remove);
    }

    #[test]
    fn counter_defaults_to_two_and_trigger_without_model_fails() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let counter = spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_counter", &[("targetname", "c1")]),
        );
        assert!(matches!(
            behaviors.triggers.get(counter.id()).map(|trigger| &trigger.kind),
            Some(Q1TriggerKind::Counter { count: 2, .. })
        ));
        let fields = trigger_fields("trigger_multiple", &[]);
        let actor = server.spawn_entity(&fields).unwrap();
        assert!(build_q1_trigger(&mut server, &mut behaviors, &actor, &fields, &[trigger_model()]).is_err());
        let fields = trigger_fields("func_button", &[]);
        let actor = server.spawn_entity(&fields).unwrap();
        assert!(build_q1_button(&mut server, &mut behaviors, &actor, &fields, &[trigger_model()]).is_err());
    }

    #[test]
    fn teleport_destination_records_mangle_and_lifted_origin() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let fields = trigger_fields(
            "info_teleport_destination",
            &[("targetname", "d1"), ("origin", "100 200 300"), ("angle", "90")],
        );
        let destination = spawn_destination(&mut server, &mut behaviors, &fields);
        assert!(server.simulation().body_state(destination.id()).is_none());
        let record = behaviors.teleport_destinations.get(destination.id()).unwrap();
        assert_eq!(record.origin, vec3(100.0, 200.0, 327.0));
        assert_eq!(record.mangle, fields.angles);
        let anonymous = trigger_fields("info_teleport_destination", &[("origin", "0 0 0")]);
        let actor = server.spawn_entity(&anonymous).unwrap();
        assert!(q1_note_teleport_destination(&mut behaviors, actor.id(), &anonymous).is_err());
        assert!(!behaviors.teleport_destinations.contains_key(actor.id()));
    }

    #[test]
    fn trigger_teleport_requires_target() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let fields = trigger_fields("trigger_teleport", &[("model", "*0"), ("target", "d1")]);
        let teleporter = server.spawn_entity(&fields).unwrap();
        build_q1_trigger(&mut server, &mut behaviors, &teleporter, &fields, &[trigger_model()]).unwrap();
        assert!(server.triggers_mut().is_trigger(teleporter.id()));
        assert!(matches!(
            behaviors.triggers.get(teleporter.id()).map(|trigger| &trigger.kind),
            Some(Q1TriggerKind::Teleport {
                player_only: false,
                targeted: false,
                ..
            })
        ));
        let fields = trigger_fields("trigger_teleport", &[("model", "*0")]);
        let actor = server.spawn_entity(&fields).unwrap();
        assert!(build_q1_trigger(&mut server, &mut behaviors, &actor, &fields, &[trigger_model()]).is_err());
        assert!(!behaviors.triggers.contains_key(actor.id()));
    }

    #[test]
    fn player_teleport_moves_fires_fogs_and_teledeath() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let destination = spawn_destination(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "info_teleport_destination",
                &[("targetname", "d1"), ("origin", "100 200 300"), ("angle", "90")],
            ),
        );
        // Same-named relay after the destination: `find` order keeps the
        // destination first, and `SUB_UseTargets` still fires the relay.
        spawn_use_point(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_relay", &[("targetname", "d1"), ("message", "ported")]),
        );
        let teleporter = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_teleport", &[("model", "*0"), ("target", "d1")]),
        );
        let floor = server
            .simulation_mut()
            .spawn(
                qa_core::identity::ProviderId::new("q1", "test"),
                "q1:test_floor",
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        let player = spawn_player_on(&mut server, vec3(32.0, 32.0, 32.0), Some(floor.id().clone()));
        admit_player(&mut behaviors, &player);
        touch(&mut server, &mut behaviors, teleporter.id(), player.id());

        let record = behaviors.teleport_destinations.get(destination.id()).unwrap().clone();
        let body = server.simulation().body_state(player.id()).unwrap();
        assert_eq!(body.origin, record.origin);
        assert_eq!(body.angles, record.mangle);
        assert!(body.ground.is_none());
        let forward = angle_vectors(record.mangle).forward;
        assert_eq!(
            body.velocity,
            vec3(forward.x * 300.0, forward.y * 300.0, forward.z * 300.0)
        );
        assert_eq!(behaviors.player_forces.len(), 1);
        let force = &behaviors.player_forces[0];
        assert_eq!(force.actor, *player.id());
        assert_eq!(force.origin, Some(record.origin));
        assert_eq!(force.angles, Some(record.mangle));
        assert_eq!(force.velocity, Some(body.velocity));
        assert!((force.teleport_time_seconds.unwrap() - 0.7).abs() < 1e-9);
        assert_eq!(behaviors.centerprints.len(), 1);
        assert_eq!(behaviors.centerprints[0].text, "ported");
        assert_eq!(behaviors.teleport_fogs.len(), 2);
        assert_eq!(behaviors.teleport_fogs[0], vec3(32.0, 32.0, 32.0));
        assert_eq!(
            behaviors.teleport_fogs[1],
            vec3(
                record.origin.x + forward.x * 32.0,
                record.origin.y + forward.y * 32.0,
                record.origin.z + forward.z * 32.0
            )
        );
        let deaths: Vec<ActorId> = behaviors
            .triggers
            .iter()
            .filter_map(|(id, trigger)| match &trigger.kind {
                Q1TriggerKind::Teledeath { owner } if owner == player.id() => Some(id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(deaths.len(), 1);
        assert!(server.triggers_mut().is_trigger(&deaths[0]));
        let death_body = server.simulation().body_state(&deaths[0]).unwrap();
        assert_eq!(death_body.origin, record.origin);
        assert_eq!(death_body.bounds.min, vec3(-17.0, -17.0, -25.0));
        assert_eq!(death_body.bounds.max, vec3(17.0, 17.0, 33.0));
        assert!(behaviors.thinks.iter().any(|think| think.actor == deaths[0]
            && think.kind == Q1ThinkKind::Remove
            && (think.due_seconds - 0.2).abs() < 1e-9));
    }

    #[test]
    fn targeted_teleport_gates_until_used_through_live_ticks() {
        let mut server = test_server();
        register_all(&mut server);
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        spawn_destination(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields(
                "info_teleport_destination",
                &[("targetname", "d1"), ("origin", "500 0 0")],
            ),
        );
        let teleporter = spawn_brush_trigger(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields(
                "trigger_teleport",
                &[("model", "*0"), ("target", "d1"), ("targetname", "t1")],
            ),
        );
        // Player far from the volume: ticks advance the clock without the
        // sweep touching.
        let player = spawn_player(&mut server, vec3(4000.0, 4000.0, 4000.0));
        admit_player(&mut shared.borrow_mut(), &player);
        super::super::native_q1_spawns::install_q1_native(&mut server, Rc::clone(&shared));
        server.tick(SourceTime::Seconds(0.05)).unwrap();
        // Never fired: shut.
        touch_via(&mut server, &shared, teleporter.id(), player.id());
        assert_eq!(
            server.simulation().body_state(player.id()).unwrap().origin,
            vec3(4000.0, 4000.0, 4000.0)
        );
        // Fired: open; the touch teleports.
        fire_use_via(&mut server, &shared, teleporter.id(), Some(player.id()));
        touch_via(&mut server, &shared, teleporter.id(), player.id());
        assert_eq!(
            server.simulation().body_state(player.id()).unwrap().origin,
            vec3(500.0, 0.0, 27.0)
        );
        // Back across, then let the gate lapse: shut again.
        server
            .simulation_mut()
            .set_body_origin(player.id(), vec3(4000.0, 4000.0, 4000.0))
            .unwrap();
        for _ in 0..6 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        touch_via(&mut server, &shared, teleporter.id(), player.id());
        assert_eq!(
            server.simulation().body_state(player.id()).unwrap().origin,
            vec3(4000.0, 4000.0, 4000.0)
        );
    }

    fn touch_via(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        shared: &Rc<RefCell<Q1NativeBehaviors>>,
        trigger: &ActorId,
        other: &ActorId,
    ) {
        let contact = TouchContact {
            trigger: trigger.clone(),
            other: other.clone(),
        };
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_trigger_touch(&mut shared.borrow_mut(), simulation, movers, triggers, &contact);
    }

    fn fire_use_via(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        shared: &Rc<RefCell<Q1NativeBehaviors>>,
        actor: &ActorId,
        activator: Option<&ActorId>,
    ) {
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_fire_use(&mut shared.borrow_mut(), simulation, movers, triggers, actor, activator);
    }

    #[test]
    fn player_only_teleport_rejects_monsters() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_destination(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "info_teleport_destination",
                &[("targetname", "d1"), ("origin", "100 0 0")],
            ),
        );
        let gated = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "trigger_teleport",
                &[("model", "*0"), ("target", "d1"), ("spawnflags", "1")],
            ),
        );
        let open = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_teleport", &[("model", "*0"), ("target", "d1")]),
        );
        // Solid, living, but never admitted: a monster stand-in.
        let monster = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.solids.insert(monster.id().clone());
        server
            .simulation_mut()
            .set_body_velocity(monster.id(), vec3(10.0, 20.0, 30.0))
            .unwrap();
        touch(&mut server, &mut behaviors, gated.id(), monster.id());
        assert_eq!(
            server.simulation().body_state(monster.id()).unwrap().origin,
            vec3(32.0, 32.0, 32.0)
        );
        assert!(behaviors.player_forces.is_empty());
        touch(&mut server, &mut behaviors, open.id(), monster.id());
        let body = server.simulation().body_state(monster.id()).unwrap();
        assert_eq!(body.origin, vec3(100.0, 0.0, 27.0));
        assert_eq!(body.velocity, vec3(10.0, 20.0, 30.0));
        assert_eq!(behaviors.player_forces.len(), 1);
        assert_eq!(behaviors.player_forces[0].velocity, None);
        assert_eq!(behaviors.player_forces[0].teleport_time_seconds, None);
    }

    #[test]
    fn teleport_rejects_dead_nonsolid_and_dangling() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_destination(
            &mut server,
            &mut behaviors,
            &trigger_fields(
                "info_teleport_destination",
                &[("targetname", "d1"), ("origin", "100 0 0")],
            ),
        );
        let teleporter = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_teleport", &[("model", "*0"), ("target", "d1")]),
        );
        let dangling = spawn_brush_trigger(
            &mut server,
            &mut behaviors,
            &trigger_fields("trigger_teleport", &[("model", "*0"), ("target", "nowhere")]),
        );
        let player = spawn_player(&mut server, vec3(32.0, 32.0, 32.0));
        behaviors.set_player(Some(player.id().clone()));
        // Nonsolid: admitted but never linked (no `SOLID_SLIDEBOX`).
        touch(&mut server, &mut behaviors, teleporter.id(), player.id());
        assert_eq!(
            server.simulation().body_state(player.id()).unwrap().origin,
            vec3(32.0, 32.0, 32.0)
        );
        behaviors.solids.insert(player.id().clone());
        // Dead: health zero.
        server
            .simulation_mut()
            .set_combat(
                player.id(),
                CombatState {
                    health: 0.0,
                    ..CombatState::default()
                },
            )
            .unwrap();
        touch(&mut server, &mut behaviors, teleporter.id(), player.id());
        assert_eq!(
            server.simulation().body_state(player.id()).unwrap().origin,
            vec3(32.0, 32.0, 32.0)
        );
        // Living solid, but the target dangles: full no-op, no firing.
        server
            .simulation_mut()
            .set_combat(player.id(), CombatState::default())
            .unwrap();
        touch(&mut server, &mut behaviors, dangling.id(), player.id());
        assert_eq!(
            server.simulation().body_state(player.id()).unwrap().origin,
            vec3(32.0, 32.0, 32.0)
        );
        assert!(behaviors.teleport_fogs.is_empty());
        assert!(behaviors.player_forces.is_empty());
    }

    #[test]
    fn teledeath_telefrags_victim_spares_owner_then_removes() {
        let mut server = test_server();
        register_all(&mut server);
        let shared = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        spawn_destination(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields(
                "info_teleport_destination",
                &[("targetname", "d1"), ("origin", "100 0 0")],
            ),
        );
        let teleporter = spawn_brush_trigger(
            &mut server,
            &mut shared.borrow_mut(),
            &trigger_fields("trigger_teleport", &[("model", "*0"), ("target", "d1")]),
        );
        let player = spawn_player(&mut server, vec3(4000.0, 4000.0, 4000.0));
        admit_player(&mut shared.borrow_mut(), &player);
        let victim = spawn_player(&mut server, vec3(100.0, 0.0, 27.0));
        shared.borrow_mut().solids.insert(victim.id().clone());
        super::super::native_q1_spawns::install_q1_native(&mut server, Rc::clone(&shared));
        touch_via(&mut server, &shared, teleporter.id(), player.id());
        let death = shared
            .borrow()
            .triggers
            .iter()
            .find_map(|(id, trigger)| match &trigger.kind {
                Q1TriggerKind::Teledeath { .. } => Some(id.clone()),
                _ => None,
            })
            .unwrap();
        // Owner immune, victim telefragged.
        touch_via(&mut server, &shared, &death, player.id());
        assert_eq!(server.simulation().combat_state(player.id()).unwrap().health, 100.0);
        touch_via(&mut server, &shared, &death, victim.id());
        assert!(server.simulation().combat_state(victim.id()).unwrap().health <= 0.0);
        for _ in 0..6 {
            server.tick(SourceTime::Seconds(0.05)).unwrap();
        }
        assert!(simulation_actor_gone(&server, &death));
        assert!(!shared.borrow().triggers.contains_key(&death));
    }
}
