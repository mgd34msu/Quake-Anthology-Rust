//! Quake II mover checkpoints ported from `src/persistence/q2-movers.ts`.

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, num, obj, str, SaveJson, SaveReader,
};

use super::super::PersistenceError;

/// Saved linear-motion curve.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MotionCurve {
    /// Positions.
    pub positions: Vec<f64>,
    /// Frame.
    pub frame: f64,
    /// Subframe.
    pub subframe: f64,
    /// Subframes.
    pub subframes: f64,
}

/// Saved linear-motion state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2LinearMotionState {
    /// Direction.
    pub direction: Vec3,
    /// Destination.
    pub destination: Vec3,
    /// Reference.
    pub reference: Vec3,
    /// Remaining distance.
    pub remaining: f64,
    /// Current speed.
    pub current_speed: f64,
    /// Move speed.
    pub move_speed: f64,
    /// Next speed.
    pub next_speed: f64,
    /// Deceleration distance.
    pub decel_distance: f64,
    /// Done callback.
    pub done: String,
    /// Curve.
    pub curve: Option<Q2MotionCurve>,
}

/// Saved linear-motion entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2LinearMotionEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// State.
    pub state: Q2LinearMotionState,
}

/// Read a linear-motion checkpoint list.
pub fn read_q2_linear_motion_checkpoint(reader: SaveReader) -> Result<Vec<Q2LinearMotionEntry>, PersistenceError> {
    reader.list(|value| -> Result<Q2LinearMotionEntry, PersistenceError> {
        let state = value.field("state");
        let number = |key: &str| state.field(key).number().map_err(PersistenceError::from);
        Ok(Q2LinearMotionEntry {
            actor: read_saved_actor(value.field("actor"))?,
            state: Q2LinearMotionState {
                direction: read_vector(state.field("direction"))?,
                destination: read_vector(state.field("destination"))?,
                reference: read_vector(state.field("reference"))?,
                remaining: number("remaining")?,
                current_speed: number("currentSpeed")?,
                move_speed: number("moveSpeed")?,
                next_speed: number("nextSpeed")?,
                decel_distance: number("decelDistance")?,
                done: state.field("done").string()?,
                curve: state
                    .field("curve")
                    .nullable(|curve| -> Result<Q2MotionCurve, PersistenceError> {
                        Ok(Q2MotionCurve {
                            positions: curve
                                .field("positions")
                                .list(|position| position.number().map_err(PersistenceError::from))?,
                            frame: curve.field("frame").number()?,
                            subframe: curve.field("subframe").number()?,
                            subframes: curve.field("subframes").number()?,
                        })
                    })?,
            },
        })
    })
}

/// Write a linear-motion checkpoint list.
#[must_use]
pub fn write_q2_linear_motion_checkpoint(entries: &[Q2LinearMotionEntry]) -> SaveJson {
    arr(entries
        .iter()
        .map(|entry| {
            obj(vec![
                ("actor", write_saved_actor(entry.actor)),
                (
                    "state",
                    obj(vec![
                        ("direction", write_vector(entry.state.direction)),
                        ("destination", write_vector(entry.state.destination)),
                        ("reference", write_vector(entry.state.reference)),
                        ("remaining", num(entry.state.remaining)),
                        ("currentSpeed", num(entry.state.current_speed)),
                        ("moveSpeed", num(entry.state.move_speed)),
                        ("nextSpeed", num(entry.state.next_speed)),
                        ("decelDistance", num(entry.state.decel_distance)),
                        ("done", str(&entry.state.done)),
                        (
                            "curve",
                            entry.state.curve.as_ref().map_or(SaveJson::Null, |curve| {
                                obj(vec![
                                    (
                                        "positions",
                                        arr(curve.positions.iter().map(|position| num(*position)).collect()),
                                    ),
                                    ("frame", num(curve.frame)),
                                    ("subframe", num(curve.subframe)),
                                    ("subframes", num(curve.subframes)),
                                ])
                            }),
                        ),
                    ]),
                ),
            ])
        })
        .collect())
}

/// Saved angular-motion entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2AngularMotionEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// Destination.
    pub destination: Vec3,
    /// Speed.
    pub speed: f64,
    /// Done callback.
    pub done: String,
}

/// Read an angular-motion checkpoint list.
pub fn read_q2_angular_motion_checkpoint(reader: SaveReader) -> Result<Vec<Q2AngularMotionEntry>, PersistenceError> {
    reader.list(|value| -> Result<Q2AngularMotionEntry, PersistenceError> {
        Ok(Q2AngularMotionEntry {
            actor: read_saved_actor(value.field("actor"))?,
            destination: read_vector(value.field("destination"))?,
            speed: value.field("speed").number()?,
            done: value.field("done").string()?,
        })
    })
}

/// Write an angular-motion checkpoint list.
#[must_use]
pub fn write_q2_angular_motion_checkpoint(entries: &[Q2AngularMotionEntry]) -> SaveJson {
    arr(entries
        .iter()
        .map(|entry| {
            obj(vec![
                ("actor", write_saved_actor(entry.actor)),
                ("destination", write_vector(entry.destination)),
                ("speed", num(entry.speed)),
                ("done", str(&entry.done)),
            ])
        })
        .collect())
}

/// Saved door state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2DoorState {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Distance.
    pub distance: f64,
    /// Button flag.
    pub button: bool,
    /// Angular flag.
    pub angular: bool,
    /// Water flag.
    pub water: bool,
    /// Safe direction.
    pub safe_direction: Vec3,
    /// Water divisor.
    pub water_divisor: f64,
    /// Reversed flag.
    pub reversed: bool,
    /// Activated flag.
    pub activated: bool,
    /// Phase.
    pub phase: String,
    /// Debounce time.
    pub debounce: f64,
}

/// Saved door entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2DoorEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// Master.
    pub master: SavedActorId,
    /// Team.
    pub team: Vec<SavedActorId>,
    /// State.
    pub state: Q2DoorState,
}

/// Saved train entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TrainEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// Destination.
    pub destination: Option<SavedActorId>,
    /// Debounce time.
    pub debounce: f64,
    /// Ship flag.
    pub ship: bool,
}

/// Q2 movers checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MoversCheckpoint {
    /// Doors.
    pub doors: Vec<Q2DoorEntry>,
    /// Trains.
    pub trains: Vec<Q2TrainEntry>,
    /// Linear motion.
    pub linear: Vec<Q2LinearMotionEntry>,
    /// Angular motion.
    pub angular: Vec<Q2AngularMotionEntry>,
}

/// Read a Q2 movers checkpoint.
pub fn read_q2_movers_checkpoint(reader: SaveReader) -> Result<Q2MoversCheckpoint, PersistenceError> {
    Ok(Q2MoversCheckpoint {
        doors: reader
            .field("doors")
            .list(|value| -> Result<Q2DoorEntry, PersistenceError> {
                let state = value.field("state");
                Ok(Q2DoorEntry {
                    actor: read_saved_actor(value.field("actor"))?,
                    master: read_saved_actor(value.field("master"))?,
                    team: value
                        .field("team")
                        .list(|member| read_saved_actor(member).map_err(PersistenceError::from))?,
                    state: Q2DoorState {
                        start: read_vector(state.field("start"))?,
                        end: read_vector(state.field("end"))?,
                        distance: state.field("distance").number()?,
                        button: state.field("button").boolean()?,
                        angular: state.field("angular").boolean()?,
                        water: state.field("water").boolean()?,
                        safe_direction: read_vector(state.field("safeDirection"))?,
                        water_divisor: state.field("waterDivisor").number()?,
                        reversed: state.field("reversed").boolean()?,
                        activated: state.field("activated").boolean()?,
                        phase: state.field("phase").choice_str(&["bottom", "up", "top", "down"])?,
                        debounce: state.field("debounce").number()?,
                    },
                })
            })?,
        trains: reader
            .field("trains")
            .list(|value| -> Result<Q2TrainEntry, PersistenceError> {
                Ok(Q2TrainEntry {
                    actor: read_saved_actor(value.field("actor"))?,
                    destination: value
                        .field("destination")
                        .nullable(|item| read_saved_actor(item).map_err(PersistenceError::from))?,
                    debounce: value.field("debounce").number()?,
                    ship: value.field("ship").boolean()?,
                })
            })?,
        linear: read_q2_linear_motion_checkpoint(reader.field("linear"))?,
        angular: read_q2_angular_motion_checkpoint(reader.field("angular"))?,
    })
}

/// Write a Q2 movers checkpoint.
#[must_use]
pub fn write_q2_movers_checkpoint(checkpoint: &Q2MoversCheckpoint) -> SaveJson {
    obj(vec![
        (
            "doors",
            arr(checkpoint
                .doors
                .iter()
                .map(|door| {
                    obj(vec![
                        ("actor", write_saved_actor(door.actor)),
                        ("master", write_saved_actor(door.master)),
                        (
                            "team",
                            arr(door.team.iter().map(|actor| write_saved_actor(*actor)).collect()),
                        ),
                        (
                            "state",
                            obj(vec![
                                ("start", write_vector(door.state.start)),
                                ("end", write_vector(door.state.end)),
                                ("distance", num(door.state.distance)),
                                ("button", boolean(door.state.button)),
                                ("angular", boolean(door.state.angular)),
                                ("water", boolean(door.state.water)),
                                ("safeDirection", write_vector(door.state.safe_direction)),
                                ("waterDivisor", num(door.state.water_divisor)),
                                ("reversed", boolean(door.state.reversed)),
                                ("activated", boolean(door.state.activated)),
                                ("phase", str(&door.state.phase)),
                                ("debounce", num(door.state.debounce)),
                            ]),
                        ),
                    ])
                })
                .collect()),
        ),
        (
            "trains",
            arr(checkpoint
                .trains
                .iter()
                .map(|train| {
                    obj(vec![
                        ("actor", write_saved_actor(train.actor)),
                        (
                            "destination",
                            train.destination.map_or(SaveJson::Null, write_saved_actor),
                        ),
                        ("debounce", num(train.debounce)),
                        ("ship", boolean(train.ship)),
                    ])
                })
                .collect()),
        ),
        ("linear", write_q2_linear_motion_checkpoint(&checkpoint.linear)),
        ("angular", write_q2_angular_motion_checkpoint(&checkpoint.angular)),
    ])
}

/// Encode a Q2 movers checkpoint.
#[must_use]
pub fn encode_q2_movers_checkpoint(checkpoint: &Q2MoversCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_movers_checkpoint(checkpoint))
}

/// Decode a Q2 movers checkpoint.
pub fn decode_q2_movers_checkpoint(bytes: &[u8]) -> Result<Q2MoversCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_movers_checkpoint(SaveReader::at(&payload, "q2-movers"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero() -> Vec3 {
        Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    }

    #[test]
    fn movers_round_trip() {
        let checkpoint = Q2MoversCheckpoint {
            doors: vec![Q2DoorEntry {
                actor: SavedActorId { slot: 1, generation: 0 },
                master: SavedActorId { slot: 1, generation: 0 },
                team: Vec::new(),
                state: Q2DoorState {
                    start: zero(),
                    end: zero(),
                    distance: 64.0,
                    button: false,
                    angular: false,
                    water: false,
                    safe_direction: zero(),
                    water_divisor: 1.0,
                    reversed: false,
                    activated: false,
                    phase: "bottom".to_string(),
                    debounce: 0.0,
                },
            }],
            trains: vec![Q2TrainEntry {
                actor: SavedActorId { slot: 2, generation: 0 },
                destination: None,
                debounce: 0.0,
                ship: false,
            }],
            linear: vec![Q2LinearMotionEntry {
                actor: SavedActorId { slot: 1, generation: 0 },
                state: Q2LinearMotionState {
                    direction: zero(),
                    destination: zero(),
                    reference: zero(),
                    remaining: 0.0,
                    current_speed: 0.0,
                    move_speed: 100.0,
                    next_speed: 0.0,
                    decel_distance: 0.0,
                    done: "door_done".to_string(),
                    curve: Some(Q2MotionCurve {
                        positions: vec![0.0, 1.0],
                        frame: 0.0,
                        subframe: 0.0,
                        subframes: 2.0,
                    }),
                },
            }],
            angular: vec![Q2AngularMotionEntry {
                actor: SavedActorId { slot: 3, generation: 0 },
                destination: zero(),
                speed: 45.0,
                done: "rotate_done".to_string(),
            }],
        };
        let bytes = encode_q2_movers_checkpoint(&checkpoint);
        assert_eq!(decode_q2_movers_checkpoint(&bytes).unwrap(), checkpoint);
    }
}
