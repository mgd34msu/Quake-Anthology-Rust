//! Port of `src/compat/qc/builtins.ts` (Quake/QuakeWorld `pr_cmds.c` and
//! rerelease `defs.qc`, GPL-2.0-or-later): pure QuakeC builtins plus the
//! host-builtin requirement table.
//!
//! Local mirrors: [`QcRandomSource`] mirrors the `RandomSource` contract
//! (`src/contracts/numeric.ts`); [`QcHostBuiltinName`] mirrors the donor's
//! host-builtin name union. The session RNG stays injectable and
//! checkpointable by its owner; builtin 7 only draws from it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::math::{vec3, Vec3};

use super::machine::{QcBuiltin, QcBuiltinRegistry, QcMachine};
use crate::error::GuestError;

/// Host flavor selecting the builtin requirement table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcHostKind {
    /// NetQuake.
    Netquake,
    /// QuakeWorld (no `particle`, plus `logfrag`/`infokey`/`multicast`).
    Quakeworld,
    /// Rerelease (plus `setcolor` and named `ex_*` builtins).
    Rerelease,
}

/// Host-owned builtin name. Pure builtins are installed by number in
/// [`create_qc_builtins`]; these names are only bound when the host
/// supplies them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcHostBuiltinName {
    /// Set entity origin.
    Setorigin,
    /// Set entity model.
    Setmodel,
    /// Set entity size.
    Setsize,
    /// Debug break.
    Break,
    /// Start a sound.
    Sound,
    /// Object error.
    Objerror,
    /// Spawn an entity.
    Spawn,
    /// Remove an entity.
    Remove,
    /// Trace a line.
    Traceline,
    /// Check for a client.
    Checkclient,
    /// Precache a sound.
    PrecacheSound,
    /// Precache a model.
    PrecacheModel,
    /// Stuff a client command.
    Stuffcmd,
    /// Find entities in radius.
    Findradius,
    /// Broadcast print.
    Bprint,
    /// Single-client print.
    Sprint,
    /// Developer print.
    Dprint,
    /// Dump an entity.
    Coredump,
    /// Print an entity.
    Eprint,
    /// Walk move.
    Walkmove,
    /// Drop to floor.
    Droptofloor,
    /// Set a light style.
    Lightstyle,
    /// Check bottom.
    Checkbottom,
    /// Point contents.
    Pointcontents,
    /// Aim vector.
    Aim,
    /// Read a cvar.
    Cvar,
    /// Run a local command.
    Localcmd,
    /// Spawn a particle.
    Particle,
    /// Change yaw.
    Changeyaw,
    /// Write a byte.
    WriteByte,
    /// Write a char.
    WriteChar,
    /// Write a short.
    WriteShort,
    /// Write a long.
    WriteLong,
    /// Write a coord.
    WriteCoord,
    /// Write an angle.
    WriteAngle,
    /// Write a string.
    WriteString,
    /// Write an entity.
    WriteEntity,
    /// Move to goal.
    Movetogoal,
    /// Precache a file.
    PrecacheFile,
    /// Make static.
    Makestatic,
    /// Change level.
    Changelevel,
    /// Set a cvar.
    CvarSet,
    /// Center print.
    Centerprint,
    /// Ambient sound.
    Ambientsound,
    /// Set spawn parms.
    Setspawnparms,
    /// Log a frag (QuakeWorld).
    Logfrag,
    /// Info key (QuakeWorld).
    Infokey,
    /// Multicast (QuakeWorld).
    Multicast,
    /// Set color (rerelease).
    Setcolor,
    /// Rerelease broadcast print.
    ExBprint,
    /// Rerelease single print.
    ExSprint,
    /// Rerelease center print.
    ExCenterprint,
    /// Rerelease finale finished.
    ExFinaleFinished,
    /// Rerelease local sound.
    ExLocalsound,
    /// Rerelease draw point.
    ExDrawPoint,
    /// Rerelease draw line.
    ExDrawLine,
    /// Rerelease draw arrow.
    ExDrawArrow,
    /// Rerelease draw ray.
    ExDrawRay,
    /// Rerelease draw circle.
    ExDrawCircle,
    /// Rerelease draw bounds.
    ExDrawBounds,
    /// Rerelease draw world text.
    ExDrawWorldtext,
    /// Rerelease draw sphere.
    ExDrawSphere,
    /// Rerelease draw cylinder.
    ExDrawCylinder,
    /// Rerelease bot move to point.
    ExBotMovetopoint,
    /// Rerelease bot follow entity.
    ExBotFollowentity,
    /// Rerelease check player EX flags.
    ExCheckPlayerEXFlags,
    /// Rerelease walk path to goal.
    ExWalkpathtogoal,
    /// Rerelease prompt.
    ExPrompt,
    /// Rerelease prompt choice.
    ExPromptchoice,
    /// Rerelease clear prompt.
    ExClearprompt,
}

impl QcHostBuiltinName {
    /// Source spelling of the builtin name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Setorigin => "setorigin",
            Self::Setmodel => "setmodel",
            Self::Setsize => "setsize",
            Self::Break => "break",
            Self::Sound => "sound",
            Self::Objerror => "objerror",
            Self::Spawn => "spawn",
            Self::Remove => "remove",
            Self::Traceline => "traceline",
            Self::Checkclient => "checkclient",
            Self::PrecacheSound => "precache_sound",
            Self::PrecacheModel => "precache_model",
            Self::Stuffcmd => "stuffcmd",
            Self::Findradius => "findradius",
            Self::Bprint => "bprint",
            Self::Sprint => "sprint",
            Self::Dprint => "dprint",
            Self::Coredump => "coredump",
            Self::Eprint => "eprint",
            Self::Walkmove => "walkmove",
            Self::Droptofloor => "droptofloor",
            Self::Lightstyle => "lightstyle",
            Self::Checkbottom => "checkbottom",
            Self::Pointcontents => "pointcontents",
            Self::Aim => "aim",
            Self::Cvar => "cvar",
            Self::Localcmd => "localcmd",
            Self::Particle => "particle",
            Self::Changeyaw => "changeyaw",
            Self::WriteByte => "WriteByte",
            Self::WriteChar => "WriteChar",
            Self::WriteShort => "WriteShort",
            Self::WriteLong => "WriteLong",
            Self::WriteCoord => "WriteCoord",
            Self::WriteAngle => "WriteAngle",
            Self::WriteString => "WriteString",
            Self::WriteEntity => "WriteEntity",
            Self::Movetogoal => "movetogoal",
            Self::PrecacheFile => "precache_file",
            Self::Makestatic => "makestatic",
            Self::Changelevel => "changelevel",
            Self::CvarSet => "cvar_set",
            Self::Centerprint => "centerprint",
            Self::Ambientsound => "ambientsound",
            Self::Setspawnparms => "setspawnparms",
            Self::Logfrag => "logfrag",
            Self::Infokey => "infokey",
            Self::Multicast => "multicast",
            Self::Setcolor => "setcolor",
            Self::ExBprint => "ex_bprint",
            Self::ExSprint => "ex_sprint",
            Self::ExCenterprint => "ex_centerprint",
            Self::ExFinaleFinished => "ex_finaleFinished",
            Self::ExLocalsound => "ex_localsound",
            Self::ExDrawPoint => "ex_draw_point",
            Self::ExDrawLine => "ex_draw_line",
            Self::ExDrawArrow => "ex_draw_arrow",
            Self::ExDrawBounds => "ex_draw_bounds",
            Self::ExDrawCircle => "ex_draw_circle",
            Self::ExDrawRay => "ex_draw_ray",
            Self::ExDrawWorldtext => "ex_draw_worldtext",
            Self::ExDrawSphere => "ex_draw_sphere",
            Self::ExDrawCylinder => "ex_draw_cylinder",
            Self::ExBotMovetopoint => "ex_bot_movetopoint",
            Self::ExBotFollowentity => "ex_bot_followentity",
            Self::ExCheckPlayerEXFlags => "ex_CheckPlayerEXFlags",
            Self::ExWalkpathtogoal => "ex_walkpathtogoal",
            Self::ExPrompt => "ex_prompt",
            Self::ExPromptchoice => "ex_promptchoice",
            Self::ExClearprompt => "ex_clearprompt",
        }
    }
}

/// One host-builtin requirement: builtin number (or `None` for named
/// rerelease builtins) plus its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QcBuiltinRequirement {
    /// Builtin number, or `None` for named builtins.
    pub number: Option<i32>,
    /// Builtin name.
    pub name: QcHostBuiltinName,
}

/// Session-owned random source for builtin 7. Mirrors the donor
/// `RandomSource` contract; checkpointing stays with the owner.
pub trait QcRandomSource {
    /// Next raw integer draw.
    fn next_integer(&mut self) -> i32;
    /// Next unit draw in `[0, 1)`.
    fn next_unit(&mut self) -> f64;
}

impl QcRandomSource for qa_core::rng::Qrand {
    fn next_integer(&mut self) -> i32 {
        self.next_integer()
    }

    fn next_unit(&mut self) -> f64 {
        self.next_unit()
    }
}

/// Shared session RNG handle for builtin services.
pub type QcSharedRandom = Rc<RefCell<dyn QcRandomSource>>;

/// Services wired into the builtin table.
pub struct QcBuiltinServices {
    /// Host flavor.
    pub kind: QcHostKind,
    /// Session RNG (builtin 7 stays unbound without one).
    pub random: Option<QcSharedRandom>,
    /// Free-entity predicate (builtins 18/47 stay unbound without one).
    pub is_free_entity: Option<Rc<dyn Fn(u32) -> bool>>,
    /// Entity pre-pass before find/next enumeration.
    pub prepare_entities: Option<Rc<dyn Fn()>>,
    /// Host-owned builtin implementations by name.
    pub host: Option<HashMap<QcHostBuiltinName, QcBuiltin>>,
    /// Advertised extension strings (builtin 99).
    pub extensions: HashSet<String>,
}

impl QcBuiltinServices {
    /// Services for `kind` with no host bindings or RNG.
    #[must_use]
    pub fn new(kind: QcHostKind) -> Self {
        Self {
            kind,
            random: None,
            is_free_entity: None,
            prepare_entities: None,
            host: None,
            extensions: HashSet::new(),
        }
    }
}

const HOST_NUMBERS: &[(i32, QcHostBuiltinName)] = &[
    (2, QcHostBuiltinName::Setorigin),
    (3, QcHostBuiltinName::Setmodel),
    (4, QcHostBuiltinName::Setsize),
    (6, QcHostBuiltinName::Break),
    (8, QcHostBuiltinName::Sound),
    (11, QcHostBuiltinName::Objerror),
    (14, QcHostBuiltinName::Spawn),
    (15, QcHostBuiltinName::Remove),
    (16, QcHostBuiltinName::Traceline),
    (17, QcHostBuiltinName::Checkclient),
    (19, QcHostBuiltinName::PrecacheSound),
    (20, QcHostBuiltinName::PrecacheModel),
    (21, QcHostBuiltinName::Stuffcmd),
    (22, QcHostBuiltinName::Findradius),
    (23, QcHostBuiltinName::Bprint),
    (24, QcHostBuiltinName::Sprint),
    (25, QcHostBuiltinName::Dprint),
    (28, QcHostBuiltinName::Coredump),
    (31, QcHostBuiltinName::Eprint),
    (32, QcHostBuiltinName::Walkmove),
    (34, QcHostBuiltinName::Droptofloor),
    (35, QcHostBuiltinName::Lightstyle),
    (40, QcHostBuiltinName::Checkbottom),
    (41, QcHostBuiltinName::Pointcontents),
    (44, QcHostBuiltinName::Aim),
    (45, QcHostBuiltinName::Cvar),
    (46, QcHostBuiltinName::Localcmd),
    (48, QcHostBuiltinName::Particle),
    (49, QcHostBuiltinName::Changeyaw),
    (52, QcHostBuiltinName::WriteByte),
    (53, QcHostBuiltinName::WriteChar),
    (54, QcHostBuiltinName::WriteShort),
    (55, QcHostBuiltinName::WriteLong),
    (56, QcHostBuiltinName::WriteCoord),
    (57, QcHostBuiltinName::WriteAngle),
    (58, QcHostBuiltinName::WriteString),
    (59, QcHostBuiltinName::WriteEntity),
    (67, QcHostBuiltinName::Movetogoal),
    (68, QcHostBuiltinName::PrecacheFile),
    (69, QcHostBuiltinName::Makestatic),
    (70, QcHostBuiltinName::Changelevel),
    (72, QcHostBuiltinName::CvarSet),
    (73, QcHostBuiltinName::Centerprint),
    (74, QcHostBuiltinName::Ambientsound),
    (75, QcHostBuiltinName::PrecacheModel),
    (76, QcHostBuiltinName::PrecacheSound),
    (77, QcHostBuiltinName::PrecacheFile),
    (78, QcHostBuiltinName::Setspawnparms),
];

const RERELEASE_NAMES: &[QcHostBuiltinName] = &[
    QcHostBuiltinName::ExBprint,
    QcHostBuiltinName::ExSprint,
    QcHostBuiltinName::ExCenterprint,
    QcHostBuiltinName::ExFinaleFinished,
    QcHostBuiltinName::ExLocalsound,
    QcHostBuiltinName::ExDrawPoint,
    QcHostBuiltinName::ExDrawLine,
    QcHostBuiltinName::ExDrawArrow,
    QcHostBuiltinName::ExDrawRay,
    QcHostBuiltinName::ExDrawCircle,
    QcHostBuiltinName::ExDrawBounds,
    QcHostBuiltinName::ExDrawWorldtext,
    QcHostBuiltinName::ExDrawSphere,
    QcHostBuiltinName::ExDrawCylinder,
    QcHostBuiltinName::ExBotMovetopoint,
    QcHostBuiltinName::ExBotFollowentity,
    QcHostBuiltinName::ExCheckPlayerEXFlags,
    QcHostBuiltinName::ExWalkpathtogoal,
    QcHostBuiltinName::ExPrompt,
    QcHostBuiltinName::ExPromptchoice,
    QcHostBuiltinName::ExClearprompt,
];

/// Host builtins the chosen host must supply.
pub fn qc_host_requirements(kind: QcHostKind) -> Vec<QcBuiltinRequirement> {
    let mut requirements: Vec<QcBuiltinRequirement> = HOST_NUMBERS
        .iter()
        .filter(|(number, _)| kind != QcHostKind::Quakeworld || *number != 48)
        .map(|(number, name)| QcBuiltinRequirement {
            number: Some(*number),
            name: *name,
        })
        .collect();
    if kind == QcHostKind::Quakeworld {
        requirements.push(QcBuiltinRequirement {
            number: Some(79),
            name: QcHostBuiltinName::Logfrag,
        });
        requirements.push(QcBuiltinRequirement {
            number: Some(80),
            name: QcHostBuiltinName::Infokey,
        });
        requirements.push(QcBuiltinRequirement {
            number: Some(82),
            name: QcHostBuiltinName::Multicast,
        });
    }
    if kind == QcHostKind::Rerelease {
        requirements.push(QcBuiltinRequirement {
            number: Some(401),
            name: QcHostBuiltinName::Setcolor,
        });
        for name in RERELEASE_NAMES {
            requirements.push(QcBuiltinRequirement {
                number: None,
                name: *name,
            });
        }
    }
    requirements
}

fn vector_length(machine: &QcMachine, vector: Vec3) -> f64 {
    let numeric = machine.numeric();
    let (x, y, z) = (f64::from(vector.x), f64::from(vector.y), f64::from(vector.z));
    numeric.sqrt(numeric.add(numeric.add(numeric.mul(x, x), numeric.mul(y, y)), numeric.mul(z, z)))
}

fn yaw(vector: Vec3) -> f32 {
    if vector.x == 0.0 && vector.y == 0.0 {
        return 0.0;
    }
    let angle = (f64::from(vector.y).atan2(f64::from(vector.x)) * 180.0 / std::f64::consts::PI).trunc() as i32;
    (if angle < 0 { angle + 360 } else { angle }) as f32
}

fn format_float(value: f64) -> String {
    format!("{value:>5.1}")
}

fn make_vectors(machine: &mut QcMachine) -> Result<(), GuestError> {
    let angles = machine.arg_vector(0)?;
    let numeric = machine.numeric();
    let y = f64::from(angles.y) * std::f64::consts::PI / 180.0;
    let p = f64::from(angles.x) * std::f64::consts::PI / 180.0;
    let r = f64::from(angles.z) * std::f64::consts::PI / 180.0;
    let sy = f64::from(numeric.store(y.sin()));
    let cy = f64::from(numeric.store(y.cos()));
    let sp = f64::from(numeric.store(p.sin()));
    let cp = f64::from(numeric.store(p.cos()));
    let sr = f64::from(numeric.store(r.sin()));
    let cr = f64::from(numeric.store(r.cos()));
    let forward = machine.global_offset("v_forward")?;
    let right = machine.global_offset("v_right")?;
    let up = machine.global_offset("v_up")?;
    machine
        .globals_mut()
        .set_vector(
            forward,
            vec3(numeric.mul(cp, cy) as f32, numeric.mul(cp, sy) as f32, (-sp) as f32),
        )
        .map_err(|error| machine.fail(error.to_string()))?;
    machine
        .globals_mut()
        .set_vector(
            right,
            vec3(
                numeric.add(numeric.mul(numeric.mul(-sr, sp), cy), numeric.mul(cr, sy)) as f32,
                numeric.sub(numeric.mul(numeric.mul(-sr, sp), sy), numeric.mul(cr, cy)) as f32,
                numeric.mul(-sr, cp) as f32,
            ),
        )
        .map_err(|error| machine.fail(error.to_string()))?;
    machine
        .globals_mut()
        .set_vector(
            up,
            vec3(
                numeric.add(numeric.mul(numeric.mul(cr, sp), cy), numeric.mul(sr, sy)) as f32,
                numeric.sub(numeric.mul(numeric.mul(cr, sp), sy), numeric.mul(sr, cy)) as f32,
                numeric.mul(cr, cp) as f32,
            ),
        )
        .map_err(|error| machine.fail(error.to_string()))?;
    Ok(())
}

/// Install pure builtins plus supplied host bindings. Missing engine
/// bindings stay unbound and fail by name at call time.
pub fn create_qc_builtins(services: QcBuiltinServices) -> QcBuiltinRegistry {
    let mut numbered: HashMap<i32, QcBuiltin> = HashMap::new();
    let mut named: HashMap<String, QcBuiltin> = HashMap::new();
    numbered.insert(1, Rc::new(make_vectors));
    if let Some(random) = services.random {
        numbered.insert(
            7,
            Rc::new(move |machine: &mut QcMachine| {
                let mut source = random
                    .try_borrow_mut()
                    .map_err(|_| machine.fail("random source is already borrowed"))?;
                let draw = f64::from(source.next_integer() & 0x7fff) / f64::from(0x7fff);
                machine.return_float(draw as f32)
            }),
        );
    }
    numbered.insert(
        9,
        Rc::new(|machine: &mut QcMachine| {
            let vector = machine.arg_vector(0)?;
            let numeric = machine.numeric();
            let magnitude = vector_length(machine, vector);
            let inverse = if magnitude == 0.0 {
                0.0
            } else {
                numeric.div(1.0, magnitude)
            };
            machine.return_vector(vec3(
                numeric.mul(f64::from(vector.x), inverse) as f32,
                numeric.mul(f64::from(vector.y), inverse) as f32,
                numeric.mul(f64::from(vector.z), inverse) as f32,
            ))
        }),
    );
    numbered.insert(
        10,
        Rc::new(|machine: &mut QcMachine| {
            let message = machine.var_string(0)?;
            Err(machine.fail(message))
        }),
    );
    numbered.insert(
        12,
        Rc::new(|machine: &mut QcMachine| {
            let vector = machine.arg_vector(0)?;
            machine.return_float(vector_length(machine, vector) as f32)
        }),
    );
    numbered.insert(
        13,
        Rc::new(|machine: &mut QcMachine| {
            let vector = machine.arg_vector(0)?;
            machine.return_float(yaw(vector))
        }),
    );
    numbered.insert(
        26,
        Rc::new(|machine: &mut QcMachine| {
            let value = f64::from(machine.arg_float(0)?);
            let text = if value == value.trunc() {
                format!("{}", value.trunc() as i64)
            } else {
                format_float(value)
            };
            let reference = machine
                .strings_mut()
                .set_engine("pr_string_temp", &text, 128)
                .map_err(|error| machine.fail(error.to_string()))?;
            machine.return_int(reference)
        }),
    );
    numbered.insert(
        27,
        Rc::new(|machine: &mut QcMachine| {
            let value = machine.arg_vector(0)?;
            let text = format!(
                "'{} {} {}'",
                format_float(f64::from(value.x)),
                format_float(f64::from(value.y)),
                format_float(f64::from(value.z))
            );
            let reference = machine
                .strings_mut()
                .set_engine("pr_string_temp", &text, 128)
                .map_err(|error| machine.fail(error.to_string()))?;
            machine.return_int(reference)
        }),
    );
    numbered.insert(
        29,
        Rc::new(|machine: &mut QcMachine| {
            machine.trace_enabled = true;
            Ok(())
        }),
    );
    numbered.insert(
        30,
        Rc::new(|machine: &mut QcMachine| {
            machine.trace_enabled = false;
            Ok(())
        }),
    );
    numbered.insert(
        36,
        Rc::new(|machine: &mut QcMachine| {
            let value = f64::from(machine.arg_float(0)?);
            machine.return_float((if value > 0.0 { value + 0.5 } else { value - 0.5 }).trunc() as f32)
        }),
    );
    numbered.insert(
        37,
        Rc::new(|machine: &mut QcMachine| machine.return_float(f64::from(machine.arg_float(0)?).floor() as f32)),
    );
    numbered.insert(
        38,
        Rc::new(|machine: &mut QcMachine| machine.return_float(f64::from(machine.arg_float(0)?).ceil() as f32)),
    );
    numbered.insert(
        43,
        Rc::new(|machine: &mut QcMachine| machine.return_float(f64::from(machine.arg_float(0)?).abs() as f32)),
    );
    numbered.insert(
        51,
        Rc::new(|machine: &mut QcMachine| {
            let vector = machine.arg_vector(0)?;
            let pitch = if vector.x == 0.0 && vector.y == 0.0 {
                if vector.z > 0.0 {
                    90.0
                } else {
                    270.0
                }
            } else {
                let forward = (f64::from(vector.x).powi(2) + f64::from(vector.y).powi(2)).sqrt();
                let pitch = (f64::from(vector.z).atan2(forward) * 180.0 / std::f64::consts::PI).trunc() as i32;
                (if pitch < 0 { pitch + 360 } else { pitch }) as f32
            };
            machine.return_vector(vec3(pitch, yaw(vector), 0.0))
        }),
    );
    if let Some(is_free) = services.is_free_entity.clone() {
        let prepare = services.prepare_entities.clone();
        let prepare_next = prepare.clone();
        let is_free_next = is_free.clone();
        numbered.insert(
            18,
            Rc::new(move |machine: &mut QcMachine| {
                if let Some(prepare) = &prepare {
                    prepare();
                }
                let start_reference = machine.arg_int(0)?;
                let field = machine.arg_int(1)?;
                let matched = machine.arg_string(2)?;
                let start = machine
                    .entities()
                    .slot(start_reference)
                    .map_err(|error| machine.fail(error.to_string()))?;
                let definition = machine
                    .program()
                    .fields
                    .iter()
                    .find(|definition| {
                        definition.offset as i32 == field
                            && definition.value_type == super::program::QcValueType::String
                    })
                    .map(|definition| definition.name.clone());
                let Some(name) = definition else {
                    return Err(machine.fail("find requires a source string field"));
                };
                for slot in start + 1..machine.entities().count() as u32 {
                    if is_free(slot) {
                        continue;
                    }
                    let reference = machine
                        .entities()
                        .reference(slot)
                        .map_err(|error| machine.fail(error.to_string()))?;
                    let text = machine.entity_int(reference, &name)?;
                    if text != 0 {
                        let value = machine
                            .strings()
                            .get(text)
                            .map_err(|error| machine.fail(error.to_string()))?;
                        if value == matched {
                            return machine.return_int(reference);
                        }
                    }
                }
                machine.return_int(0)
            }),
        );
        numbered.insert(
            47,
            Rc::new(move |machine: &mut QcMachine| {
                if let Some(prepare) = &prepare_next {
                    prepare();
                }
                let start_reference = machine.arg_int(0)?;
                let start = machine
                    .entities()
                    .slot(start_reference)
                    .map_err(|error| machine.fail(error.to_string()))?;
                for slot in start + 1..machine.entities().count() as u32 {
                    if !is_free_next(slot) {
                        let reference = machine
                            .entities()
                            .reference(slot)
                            .map_err(|error| machine.fail(error.to_string()))?;
                        return machine.return_int(reference);
                    }
                }
                machine.return_int(0)
            }),
        );
    }
    if services.kind == QcHostKind::Quakeworld {
        numbered.insert(
            81,
            Rc::new(|machine: &mut QcMachine| {
                let text = machine.arg_string(0)?;
                let value = qa_core::numeric::native_atof(&text).map_err(|error| machine.fail(error.to_string()))?;
                machine.return_float(value as f32)
            }),
        );
    }
    let extensions = Rc::new(services.extensions);
    numbered.insert(
        99,
        Rc::new(move |machine: &mut QcMachine| {
            let name = machine.arg_string(0)?;
            machine.return_float(f32::from(extensions.contains(&name)))
        }),
    );
    if let Some(host) = services.host.as_ref() {
        for requirement in qc_host_requirements(services.kind) {
            let Some(binding) = host.get(&requirement.name) else {
                continue;
            };
            match requirement.number {
                Some(number) => {
                    numbered.insert(number, binding.clone());
                }
                None => {
                    named.insert(requirement.name.name().to_string(), binding.clone());
                }
            }
        }
    }
    QcBuiltinRegistry { numbered, named }
}

#[cfg(test)]
mod tests {
    use super::super::machine::QcMachineOptions;
    use super::super::program::QcOpcode;
    use super::*;
    use qa_core::numeric::NumericOps;

    fn test_machine() -> QcMachine {
        // Minimal program: 32 global words incl. v_forward/v_right/v_up,
        // one string field, and room for builtin staging.
        let mut image = Vec::new();
        let push_i32 = |image: &mut Vec<u8>, value: i32| image.extend_from_slice(&value.to_le_bytes());
        let strings = b"\0main\0file.qc\0v_forward\0v_right\0v_up\0classname\0";
        // main=1 file.qc=6 v_forward=14 v_right=24 v_up=32 classname=37
        let statements = [(QcOpcode::Done as u16, 0u16, 0u16, 0u16)];
        let globals = [(3u16, 28u16, 14i32), (3, 31, 24), (3, 34, 32)];
        let fields = [(1u16, 0u16, 37i32)];
        let functions = [(0i32, 0i32, 0i32, 0i32, 1i32, 6i32, 0i32)];
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        let mut statement_blob = Vec::new();
        for (op, a, b, c) in &statements {
            for word in [*op, *a, *b, *c] {
                statement_blob.extend_from_slice(&word.to_le_bytes());
            }
        }
        blobs.push(statement_blob);
        let mut global_blob = Vec::new();
        for (raw, at, name) in &globals {
            global_blob.extend_from_slice(&raw.to_le_bytes());
            global_blob.extend_from_slice(&at.to_le_bytes());
            global_blob.extend_from_slice(&name.to_le_bytes());
        }
        blobs.push(global_blob);
        let mut field_blob = Vec::new();
        for (raw, at, name) in &fields {
            field_blob.extend_from_slice(&raw.to_le_bytes());
            field_blob.extend_from_slice(&at.to_le_bytes());
            field_blob.extend_from_slice(&name.to_le_bytes());
        }
        blobs.push(field_blob);
        let mut function_blob = Vec::new();
        for (first, params, locals, profile, name, file, count) in &functions {
            for word in [*first, *params, *locals, *profile, *name, *file, *count] {
                function_blob.extend_from_slice(&word.to_le_bytes());
            }
            function_blob.extend_from_slice(&[0u8; 8]);
        }
        blobs.push(function_blob);
        blobs.push(strings.to_vec());
        blobs.push(vec![0u8; 37 * 4]);
        let counts = [1i32, 3, 1, 1, strings.len() as i32, 37];
        let mut offset = 60i32;
        push_i32(&mut image, 6);
        push_i32(&mut image, 5927);
        for (blob, count) in blobs.iter().zip(counts) {
            push_i32(&mut image, offset);
            push_i32(&mut image, count);
            offset += blob.len() as i32;
        }
        push_i32(&mut image, 1);
        for blob in &blobs {
            image.extend_from_slice(blob);
        }
        let program = super::super::program::load_qc_program(&image, None, "test.dat").unwrap();
        let layout = super::super::memory::QcEntityLayout {
            stride_bytes: 96 + program.entity_field_words * 4,
            variables_offset_bytes: 96,
            field_words: program.entity_field_words,
        };
        let entities = super::super::memory::QcEntityMemory::new(layout, 4, 3).unwrap();
        QcMachine::new(QcMachineOptions::new(
            program,
            NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap(),
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        ))
        .unwrap()
    }

    fn invoke(registry: &QcBuiltinRegistry, machine: &mut QcMachine, number: i32) -> Result<(), GuestError> {
        let builtin = registry
            .numbered
            .get(&number)
            .cloned()
            .ok_or_else(|| machine.fail("missing test builtin"))?;
        builtin(machine)
    }

    #[test]
    fn requirements_differ_by_host_kind() {
        let netquake = qc_host_requirements(QcHostKind::Netquake);
        let quakeworld = qc_host_requirements(QcHostKind::Quakeworld);
        let rerelease = qc_host_requirements(QcHostKind::Rerelease);
        assert!(netquake.iter().any(|requirement| requirement.number == Some(48)));
        assert!(!quakeworld.iter().any(|requirement| requirement.number == Some(48)));
        assert!(quakeworld.iter().any(|requirement| requirement.number == Some(79)));
        assert!(rerelease.iter().any(|requirement| requirement.number == Some(401)));
        assert!(rerelease
            .iter()
            .any(|requirement| requirement.number.is_none() && requirement.name == QcHostBuiltinName::ExPrompt));
    }

    #[test]
    fn pure_table_binds_expected_numbers() {
        let registry = create_qc_builtins(QcBuiltinServices::new(QcHostKind::Netquake));
        for number in [1, 9, 10, 12, 13, 26, 27, 29, 30, 36, 37, 38, 43, 51, 99] {
            assert!(registry.numbered.contains_key(&number), "missing {number}");
        }
        assert!(!registry.numbered.contains_key(&7));
        assert!(!registry.numbered.contains_key(&18));
        let qw = create_qc_builtins(QcBuiltinServices::new(QcHostKind::Quakeworld));
        assert!(qw.numbered.contains_key(&81));
    }

    #[test]
    fn host_bindings_install_by_number_or_name() {
        let binding: QcBuiltin = Rc::new(|machine: &mut QcMachine| machine.return_float(1.0));
        let mut host = HashMap::new();
        host.insert(QcHostBuiltinName::Sound, binding.clone());
        host.insert(QcHostBuiltinName::ExPrompt, binding);
        let mut services = QcBuiltinServices::new(QcHostKind::Rerelease);
        services.host = Some(host);
        let registry = create_qc_builtins(services);
        assert!(registry.numbered.contains_key(&8));
        assert!(registry.named.contains_key("ex_prompt"));
        assert!(!registry.named.contains_key("ex_sprint"));
    }

    #[test]
    fn vector_builtins_compute() {
        assert_eq!(yaw(vec3(1.0, 0.0, 0.0)), 0.0);
        assert_eq!(yaw(vec3(0.0, 1.0, 0.0)), 90.0);
        assert_eq!(yaw(vec3(0.0, 0.0, 5.0)), 0.0);
        assert_eq!(format_float(3.25), "  3.2");
        let machine = test_machine();
        assert!((vector_length(&machine, vec3(3.0, 4.0, 0.0)) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_length_and_yaw_execute() {
        let registry = create_qc_builtins(QcBuiltinServices::new(QcHostKind::Netquake));
        let mut machine = test_machine();
        machine.globals_mut().set_vector(4, vec3(3.0, 4.0, 0.0)).unwrap();
        invoke(&registry, &mut machine, 12).unwrap();
        assert_eq!(machine.globals().float(1).unwrap(), 5.0);
        invoke(&registry, &mut machine, 9).unwrap();
        let normalized = machine.globals().vector(1).unwrap();
        assert!((normalized.x - 0.6).abs() < 1e-6);
        assert!((normalized.y - 0.8).abs() < 1e-6);
        machine.globals_mut().set_vector(4, vec3(0.0, 1.0, 0.0)).unwrap();
        invoke(&registry, &mut machine, 13).unwrap();
        assert_eq!(machine.globals().float(1).unwrap(), 90.0);
    }

    #[test]
    fn make_vectors_writes_basis() {
        let registry = create_qc_builtins(QcBuiltinServices::new(QcHostKind::Netquake));
        let mut machine = test_machine();
        machine.globals_mut().set_vector(4, vec3(0.0, 0.0, 0.0)).unwrap();
        invoke(&registry, &mut machine, 1).unwrap();
        let forward = machine
            .globals()
            .vector(machine.global_offset("v_forward").unwrap())
            .unwrap();
        assert!((forward.x - 1.0).abs() < 1e-6);
        assert!(forward.y.abs() < 1e-6);
    }

    #[test]
    fn text_and_rounding_builtins_execute() {
        let registry = create_qc_builtins(QcBuiltinServices::new(QcHostKind::Netquake));
        let mut machine = test_machine();
        machine.globals_mut().set_float(4, 42.0).unwrap();
        invoke(&registry, &mut machine, 26).unwrap();
        let reference = machine.globals().int(1).unwrap();
        assert_eq!(machine.strings().get(reference).unwrap(), "42");
        machine.globals_mut().set_float(4, 2.5).unwrap();
        invoke(&registry, &mut machine, 36).unwrap();
        assert_eq!(machine.globals().float(1).unwrap(), 3.0);
        machine.globals_mut().set_float(4, -2.5).unwrap();
        invoke(&registry, &mut machine, 37).unwrap();
        assert_eq!(machine.globals().float(1).unwrap(), -3.0);
        invoke(&registry, &mut machine, 29).unwrap();
        assert!(machine.trace_enabled);
        invoke(&registry, &mut machine, 30).unwrap();
        assert!(!machine.trace_enabled);
    }

    #[test]
    fn error_builtin_fails_with_message() {
        let registry = create_qc_builtins(QcBuiltinServices::new(QcHostKind::Netquake));
        let mut machine = test_machine();
        let error = invoke(&registry, &mut machine, 10).unwrap_err();
        assert!(error.to_string().contains("QuakeC 0:0"));
    }

    #[test]
    fn random_builtin_draws_from_session_source() {
        struct Fixed(i32);
        impl QcRandomSource for Fixed {
            fn next_integer(&mut self) -> i32 {
                self.0
            }
            fn next_unit(&mut self) -> f64 {
                0.5
            }
        }
        let random: QcSharedRandom = Rc::new(RefCell::new(Fixed(0x7fff)));
        let mut services = QcBuiltinServices::new(QcHostKind::Netquake);
        services.random = Some(random);
        let registry = create_qc_builtins(services);
        let mut machine = test_machine();
        invoke(&registry, &mut machine, 7).unwrap();
        assert_eq!(machine.globals().float(1).unwrap(), 1.0);
    }

    #[test]
    fn find_and_next_skip_free_slots() {
        let mut services = QcBuiltinServices::new(QcHostKind::Netquake);
        services.is_free_entity = Some(Rc::new(|slot| slot == 1));
        let registry = create_qc_builtins(services);
        let mut machine = test_machine();
        let classname = machine.strings_mut().allocate("monster").unwrap();
        machine.entities_mut().set_slot_int(2, 0, classname).unwrap();
        let start = machine.entities().reference(0).unwrap();
        machine.globals_mut().set_int(4, start).unwrap();
        machine.globals_mut().set_int(7, 0).unwrap();
        let wanted = machine.strings_mut().allocate("monster").unwrap();
        machine.globals_mut().set_int(10, wanted).unwrap();
        invoke(&registry, &mut machine, 18).unwrap();
        assert_eq!(
            machine.globals().int(1).unwrap(),
            machine.entities().reference(2).unwrap()
        );
        machine.globals_mut().set_int(4, start).unwrap();
        invoke(&registry, &mut machine, 47).unwrap();
        assert_eq!(
            machine.globals().int(1).unwrap(),
            machine.entities().reference(2).unwrap()
        );
    }

    #[test]
    fn checkext_reports_advertised_set() {
        let mut services = QcBuiltinServices::new(QcHostKind::Netquake);
        services.extensions.insert("DP_TEST".to_string());
        let registry = create_qc_builtins(services);
        let mut machine = test_machine();
        let yes = machine.strings_mut().allocate("DP_TEST").unwrap();
        machine.globals_mut().set_int(4, yes).unwrap();
        invoke(&registry, &mut machine, 99).unwrap();
        assert_eq!(machine.globals().float(1).unwrap(), 1.0);
        let no = machine.strings_mut().allocate("NOPE").unwrap();
        machine.globals_mut().set_int(4, no).unwrap();
        invoke(&registry, &mut machine, 99).unwrap();
        assert_eq!(machine.globals().float(1).unwrap(), 0.0);
    }
}
