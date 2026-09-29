//! Quake II weapon checkpoint ported from `src/persistence/q2-weapons.ts`.

use qa_core::identity::SavedActorId;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, num, obj, str, SaveJson, SaveReader,
};

use super::super::PersistenceError;

/// Saved weapon noise.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2NoiseCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Origin.
    pub origin: qa_core::math::Vec3,
    /// Time.
    pub time: f64,
    /// Secondary flag.
    pub secondary: bool,
}

fn read_noise(reader: SaveReader) -> Result<Q2NoiseCheckpoint, PersistenceError> {
    Ok(Q2NoiseCheckpoint {
        actor: read_saved_actor(reader.field("actor"))?,
        origin: read_vector(reader.field("origin"))?,
        time: reader.field("time").number()?,
        secondary: reader.field("secondary").boolean()?,
    })
}

fn write_noise(noise: &Q2NoiseCheckpoint) -> SaveJson {
    obj(vec![
        ("actor", write_saved_actor(noise.actor)),
        ("origin", write_vector(noise.origin)),
        ("time", num(noise.time)),
        ("secondary", boolean(noise.secondary)),
    ])
}

/// Saved weapon input.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponInput {
    /// Attack held.
    pub attack: bool,
    /// Latched attack.
    pub latched_attack: bool,
    /// Holster request.
    pub holster: bool,
    /// Angles.
    pub angles: qa_core::math::Vec3,
    /// Ducked flag.
    pub ducked: bool,
    /// Spectator flag.
    pub spectator: bool,
    /// Notarget flag.
    pub notarget: bool,
    /// Hand.
    pub hand: String,
    /// Animate player.
    pub animate_player: bool,
    /// Quad expiry.
    pub quad_until: f64,
    /// Double expiry.
    pub double_until: f64,
    /// Quad-fire expiry.
    pub quad_fire_until: f64,
    /// Haste flag.
    pub haste: bool,
    /// No stacked double.
    pub no_stack_double: bool,
    /// Instant switch.
    pub instant_switch: bool,
    /// Quick switch.
    pub quick_switch: bool,
    /// Infinite ammo.
    pub infinite_ammo: bool,
    /// Players collide.
    pub players_collide: bool,
    /// Gravity.
    pub gravity: f64,
    /// Weapon thunk.
    pub weapon_thunk: bool,
}

/// Read weapon input.
pub fn read_q2_weapon_input(reader: SaveReader) -> Result<Q2WeaponInput, PersistenceError> {
    Ok(Q2WeaponInput {
        attack: reader.field("attack").boolean()?,
        latched_attack: reader.field("latchedAttack").boolean()?,
        holster: reader.field("holster").boolean()?,
        angles: read_vector(reader.field("angles"))?,
        ducked: reader.field("ducked").boolean()?,
        spectator: reader.field("spectator").boolean()?,
        notarget: reader.field("notarget").boolean()?,
        hand: reader.field("hand").choice_str(&["right", "left", "center"])?,
        animate_player: reader.field("animatePlayer").boolean()?,
        quad_until: reader.field("quadUntil").number()?,
        double_until: reader.field("doubleUntil").number()?,
        quad_fire_until: reader.field("quadFireUntil").number()?,
        haste: reader.field("haste").boolean()?,
        no_stack_double: reader.field("noStackDouble").boolean()?,
        instant_switch: reader.field("instantSwitch").boolean()?,
        quick_switch: reader.field("quickSwitch").boolean()?,
        infinite_ammo: reader.field("infiniteAmmo").boolean()?,
        players_collide: reader.field("playersCollide").boolean()?,
        gravity: reader.field("gravity").number()?,
        weapon_thunk: reader.field("weaponThunk").boolean()?,
    })
}

fn write_weapon_input(input: &Q2WeaponInput) -> SaveJson {
    obj(vec![
        ("attack", boolean(input.attack)),
        ("latchedAttack", boolean(input.latched_attack)),
        ("holster", boolean(input.holster)),
        ("angles", write_vector(input.angles)),
        ("ducked", boolean(input.ducked)),
        ("spectator", boolean(input.spectator)),
        ("notarget", boolean(input.notarget)),
        ("hand", str(&input.hand)),
        ("animatePlayer", boolean(input.animate_player)),
        ("quadUntil", num(input.quad_until)),
        ("doubleUntil", num(input.double_until)),
        ("quadFireUntil", num(input.quad_fire_until)),
        ("haste", boolean(input.haste)),
        ("noStackDouble", boolean(input.no_stack_double)),
        ("instantSwitch", boolean(input.instant_switch)),
        ("quickSwitch", boolean(input.quick_switch)),
        ("infiniteAmmo", boolean(input.infinite_ammo)),
        ("playersCollide", boolean(input.players_collide)),
        ("gravity", num(input.gravity)),
        ("weaponThunk", boolean(input.weapon_thunk)),
    ])
}

/// Hand ammunition reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2HandReservation {
    /// No reservation.
    None,
    /// Finite reservation.
    Finite,
    /// Infinite reservation.
    Infinite,
}

/// Saved weapon state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponState {
    /// Primary handoff.
    pub primary_handoff: String,
    /// Hand reservation.
    pub hand_reservation: Q2HandReservation,
    /// Source firing flag.
    pub source_firing: bool,
    /// Weapon.
    pub weapon: Option<String>,
    /// Last weapon.
    pub last_weapon: Option<String>,
    /// Pending weapon.
    pub pending: Option<String>,
    /// Phase.
    pub phase: String,
    /// Frame.
    pub frame: f64,
    /// Think time.
    pub think_time: f64,
    /// Fire finished.
    pub fire_finished: f64,
    /// Fire buffered.
    pub fire_buffered: bool,
    /// Latched attack.
    pub latched_attack: bool,
    /// Machine-gun shots.
    pub machinegun_shots: f64,
    /// Empty sound time.
    pub empty_sound_time: f64,
    /// Grenade time.
    pub grenade_time: f64,
    /// Grenade finished.
    pub grenade_finished: f64,
    /// Grenade blew up.
    pub grenade_blew_up: bool,
    /// Kick origin.
    pub kick_origin: qa_core::math::Vec3,
    /// Kick angles.
    pub kick_angles: qa_core::math::Vec3,
    /// Kick time.
    pub kick_time: f64,
    /// Kick until.
    pub kick_until: f64,
    /// Kick duration.
    pub kick_duration: f64,
    /// Loop sound.
    pub loop_sound: String,
    /// View model.
    pub view_model: Option<String>,
    /// View skin.
    pub view_skin: f64,
    /// Last firing time.
    pub last_firing_time: f64,
    /// Gun rate.
    pub gun_rate: f64,
}

fn read_hand_reservation(
    reader: SaveReader,
    weapon: &Option<String>,
    phase: &str,
) -> Result<Q2HandReservation, PersistenceError> {
    if reader.is_missing() {
        if weapon.as_deref() == Some("grenades") && phase == "firing" {
            return Err(PersistenceError::from(
                reader.fail("legacy active hand grenade has no ammunition reservation"),
            ));
        }
        return Ok(Q2HandReservation::None);
    }
    let kind = reader.field("kind").choice_str(&["none", "finite", "infinite"])?;
    if kind != "none" && (weapon.as_deref() != Some("grenades") || phase != "firing") {
        return Err(PersistenceError::from(
            reader.fail("hand reservation requires a firing hand grenade weapon"),
        ));
    }
    Ok(match kind.as_str() {
        "finite" => Q2HandReservation::Finite,
        "infinite" => Q2HandReservation::Infinite,
        _ => Q2HandReservation::None,
    })
}

/// Read weapon state.
pub fn read_q2_weapon_state(reader: SaveReader) -> Result<Q2WeaponState, PersistenceError> {
    let weapon = reader
        .field("weapon")
        .nullable(|value| value.string().map_err(PersistenceError::from))?;
    let phase = reader
        .field("phase")
        .choice_str(&["activating", "ready", "firing", "dropping"])?;
    let kick_until = reader.field("kickUntil").number()?;
    let kick_duration = reader.field("kickDuration").number()?;
    Ok(Q2WeaponState {
        primary_handoff: if reader.field("primaryHandoff").is_missing() {
            "active".to_string()
        } else {
            reader
                .field("primaryHandoff")
                .choice_str(&["active", "holstering", "holstered"])?
        },
        hand_reservation: read_hand_reservation(reader.field("handReservation"), &weapon, &phase)?,
        source_firing: reader.field("sourceFiring").boolean()?,
        weapon,
        last_weapon: reader
            .field("lastWeapon")
            .nullable(|value| value.string().map_err(PersistenceError::from))?,
        pending: reader
            .field("pending")
            .nullable(|value| value.string().map_err(PersistenceError::from))?,
        phase,
        frame: reader.field("frame").number()?,
        think_time: reader.field("thinkTime").number()?,
        fire_finished: reader.field("fireFinished").number()?,
        fire_buffered: reader.field("fireBuffered").boolean()?,
        latched_attack: reader.field("latchedAttack").boolean()?,
        machinegun_shots: reader.field("machinegunShots").number()?,
        empty_sound_time: reader.field("emptySoundTime").number()?,
        grenade_time: reader.field("grenadeTime").number()?,
        grenade_finished: reader.field("grenadeFinished").number()?,
        grenade_blew_up: reader.field("grenadeBlewUp").boolean()?,
        kick_origin: read_vector(reader.field("kickOrigin"))?,
        kick_angles: read_vector(reader.field("kickAngles"))?,
        kick_time: if reader.field("kickTime").is_missing() {
            kick_until - kick_duration
        } else {
            reader.field("kickTime").number()?
        },
        kick_until,
        kick_duration,
        loop_sound: reader.field("loopSound").string()?,
        view_model: reader
            .field("viewModel")
            .nullable(|value| value.string().map_err(PersistenceError::from))?,
        view_skin: reader.field("viewSkin").number()?,
        last_firing_time: reader.field("lastFiringTime").number()?,
        gun_rate: reader.field("gunRate").number()?,
    })
}

fn write_weapon_state(state: &Q2WeaponState) -> SaveJson {
    obj(vec![
        ("primaryHandoff", str(&state.primary_handoff)),
        (
            "handReservation",
            obj(vec![(
                "kind",
                str(match state.hand_reservation {
                    Q2HandReservation::None => "none",
                    Q2HandReservation::Finite => "finite",
                    Q2HandReservation::Infinite => "infinite",
                }),
            )]),
        ),
        ("sourceFiring", boolean(state.source_firing)),
        (
            "weapon",
            state.weapon.as_ref().map_or(SaveJson::Null, |weapon| str(weapon)),
        ),
        (
            "lastWeapon",
            state.last_weapon.as_ref().map_or(SaveJson::Null, |weapon| str(weapon)),
        ),
        (
            "pending",
            state.pending.as_ref().map_or(SaveJson::Null, |weapon| str(weapon)),
        ),
        ("phase", str(&state.phase)),
        ("frame", num(state.frame)),
        ("thinkTime", num(state.think_time)),
        ("fireFinished", num(state.fire_finished)),
        ("fireBuffered", boolean(state.fire_buffered)),
        ("latchedAttack", boolean(state.latched_attack)),
        ("machinegunShots", num(state.machinegun_shots)),
        ("emptySoundTime", num(state.empty_sound_time)),
        ("grenadeTime", num(state.grenade_time)),
        ("grenadeFinished", num(state.grenade_finished)),
        ("grenadeBlewUp", boolean(state.grenade_blew_up)),
        ("kickOrigin", write_vector(state.kick_origin)),
        ("kickAngles", write_vector(state.kick_angles)),
        ("kickTime", num(state.kick_time)),
        ("kickUntil", num(state.kick_until)),
        ("kickDuration", num(state.kick_duration)),
        ("loopSound", str(&state.loop_sound)),
        (
            "viewModel",
            state.view_model.as_ref().map_or(SaveJson::Null, |model| str(model)),
        ),
        ("viewSkin", num(state.view_skin)),
        ("lastFiringTime", num(state.last_firing_time)),
        ("gunRate", num(state.gun_rate)),
    ])
}

/// Q2 weapons checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponsCheckpoint {
    /// Silencer charges.
    pub silencer_charges: Vec<(SavedActorId, i64)>,
    /// Source rules.
    pub source_rules: String,
    /// Registered weapons.
    pub registered: Vec<String>,
    /// Fallback order.
    pub fallback_order: Option<Vec<String>>,
    /// States.
    pub states: Vec<(SavedActorId, Q2WeaponState)>,
    /// Inputs.
    pub inputs: Vec<(SavedActorId, Q2WeaponInput)>,
    /// Noises.
    pub noises: Vec<(SavedActorId, Option<Q2NoiseCheckpoint>, Option<Q2NoiseCheckpoint>)>,
    /// Sound entity.
    pub sound_entity: Option<Q2NoiseCheckpoint>,
    /// Second sound entity.
    pub sound2_entity: Option<Q2NoiseCheckpoint>,
    /// Blaster causes.
    pub blaster_causes: Vec<(SavedActorId, i64)>,
}

/// Read a Q2 weapons checkpoint.
pub fn read_q2_weapons_checkpoint(reader: SaveReader) -> Result<Q2WeaponsCheckpoint, PersistenceError> {
    if reader.field("formatVersion").integer(i64::MIN)? != 2 {
        return Err(PersistenceError::from(
            reader.fail("unsupported Q2 weapon checkpoint format"),
        ));
    }
    Ok(Q2WeaponsCheckpoint {
        silencer_charges: reader.field("silencerCharges").list(
            |value| -> Result<(SavedActorId, i64), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    value.field("charges").integer(i64::MIN)?,
                ))
            },
        )?,
        source_rules: reader.field("sourceRules").choice_str(&["base", "ctf", "lmctf"])?,
        registered: reader
            .field("registered")
            .list(|value| value.string().map_err(PersistenceError::from))?,
        fallback_order: reader
            .field("fallbackOrder")
            .nullable(|value| value.list(|name| name.string().map_err(PersistenceError::from)))?,
        states: reader
            .field("states")
            .list(|value| -> Result<(SavedActorId, Q2WeaponState), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    read_q2_weapon_state(value.field("state"))?,
                ))
            })?,
        inputs: reader
            .field("inputs")
            .list(|value| -> Result<(SavedActorId, Q2WeaponInput), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    read_q2_weapon_input(value.field("input"))?,
                ))
            })?,
        noises: reader.field("noises").list(
            |value| -> Result<(SavedActorId, Option<Q2NoiseCheckpoint>, Option<Q2NoiseCheckpoint>), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    value.field("primary").nullable(read_noise)?,
                    value.field("secondary").nullable(read_noise)?,
                ))
            },
        )?,
        sound_entity: reader.field("soundEntity").nullable(read_noise)?,
        sound2_entity: reader.field("sound2Entity").nullable(read_noise)?,
        blaster_causes: reader.field("blasterCauses").list(
            |value| -> Result<(SavedActorId, i64), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    value.field("meansOfDeath").integer(i64::MIN)?,
                ))
            },
        )?,
    })
}

/// Write a Q2 weapons checkpoint.
#[must_use]
pub fn write_q2_weapons_checkpoint(checkpoint: &Q2WeaponsCheckpoint) -> SaveJson {
    obj(vec![
        ("formatVersion", int(2)),
        (
            "silencerCharges",
            arr(checkpoint
                .silencer_charges
                .iter()
                .map(|(actor, charges)| obj(vec![("actor", write_saved_actor(*actor)), ("charges", int(*charges))]))
                .collect()),
        ),
        ("sourceRules", str(&checkpoint.source_rules)),
        (
            "registered",
            arr(checkpoint.registered.iter().map(|name| str(name)).collect()),
        ),
        (
            "fallbackOrder",
            checkpoint.fallback_order.as_ref().map_or(SaveJson::Null, |order| {
                arr(order.iter().map(|name| str(name)).collect())
            }),
        ),
        (
            "states",
            arr(checkpoint
                .states
                .iter()
                .map(|(actor, state)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("state", write_weapon_state(state)),
                    ])
                })
                .collect()),
        ),
        (
            "inputs",
            arr(checkpoint
                .inputs
                .iter()
                .map(|(actor, input)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("input", write_weapon_input(input)),
                    ])
                })
                .collect()),
        ),
        (
            "noises",
            arr(checkpoint
                .noises
                .iter()
                .map(|(actor, primary, secondary)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("primary", primary.as_ref().map_or(SaveJson::Null, write_noise)),
                        ("secondary", secondary.as_ref().map_or(SaveJson::Null, write_noise)),
                    ])
                })
                .collect()),
        ),
        (
            "soundEntity",
            checkpoint.sound_entity.as_ref().map_or(SaveJson::Null, write_noise),
        ),
        (
            "sound2Entity",
            checkpoint.sound2_entity.as_ref().map_or(SaveJson::Null, write_noise),
        ),
        (
            "blasterCauses",
            arr(checkpoint
                .blaster_causes
                .iter()
                .map(|(actor, cause)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        ("meansOfDeath", int(*cause)),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Encode a Q2 weapons checkpoint.
#[must_use]
pub fn encode_q2_weapons_checkpoint(checkpoint: &Q2WeaponsCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_weapons_checkpoint(checkpoint))
}

/// Decode a Q2 weapons checkpoint.
pub fn decode_q2_weapons_checkpoint(bytes: &[u8]) -> Result<Q2WeaponsCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_weapons_checkpoint(SaveReader::at(&payload, "q2-weapons"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero() -> qa_core::math::Vec3 {
        qa_core::math::Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    }

    fn weapon_state() -> Q2WeaponState {
        Q2WeaponState {
            primary_handoff: "active".to_string(),
            hand_reservation: Q2HandReservation::None,
            source_firing: false,
            weapon: Some("blaster".to_string()),
            last_weapon: None,
            pending: None,
            phase: "ready".to_string(),
            frame: 0.0,
            think_time: 0.0,
            fire_finished: 0.0,
            fire_buffered: false,
            latched_attack: false,
            machinegun_shots: 0.0,
            empty_sound_time: 0.0,
            grenade_time: 0.0,
            grenade_finished: 0.0,
            grenade_blew_up: false,
            kick_origin: zero(),
            kick_angles: zero(),
            kick_time: 0.0,
            kick_until: 0.0,
            kick_duration: 0.0,
            loop_sound: String::new(),
            view_model: None,
            view_skin: 0.0,
            last_firing_time: 0.0,
            gun_rate: 1.0,
        }
    }

    #[test]
    fn weapons_round_trip() {
        let checkpoint = Q2WeaponsCheckpoint {
            silencer_charges: vec![(SavedActorId { slot: 1, generation: 0 }, 30)],
            source_rules: "base".to_string(),
            registered: vec!["blaster".to_string()],
            fallback_order: None,
            states: vec![(SavedActorId { slot: 1, generation: 0 }, weapon_state())],
            inputs: vec![(
                SavedActorId { slot: 1, generation: 0 },
                Q2WeaponInput {
                    attack: false,
                    latched_attack: false,
                    holster: false,
                    angles: zero(),
                    ducked: false,
                    spectator: false,
                    notarget: false,
                    hand: "right".to_string(),
                    animate_player: true,
                    quad_until: 0.0,
                    double_until: 0.0,
                    quad_fire_until: 0.0,
                    haste: false,
                    no_stack_double: false,
                    instant_switch: false,
                    quick_switch: false,
                    infinite_ammo: false,
                    players_collide: true,
                    gravity: 800.0,
                    weapon_thunk: false,
                },
            )],
            noises: Vec::new(),
            sound_entity: None,
            sound2_entity: None,
            blaster_causes: Vec::new(),
        };
        let bytes = encode_q2_weapons_checkpoint(&checkpoint);
        assert_eq!(decode_q2_weapons_checkpoint(&bytes).unwrap(), checkpoint);
    }

    #[test]
    fn legacy_kick_time_derives() {
        let mut state = write_weapon_state(&weapon_state());
        if let SaveJson::Object(members) = &mut state {
            members.retain(|(name, _)| name != "kickTime");
            for (name, value) in members.iter_mut() {
                if name == "kickUntil" {
                    *value = num(5.0);
                }
                if name == "kickDuration" {
                    *value = num(2.0);
                }
            }
        }
        assert_eq!(read_q2_weapon_state(SaveReader::new(&state)).unwrap().kick_time, 3.0);
    }
}
