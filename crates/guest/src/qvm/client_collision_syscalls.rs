//! Cgame collision traps.
//!
//! Provenance: `src/compat/qvm/client-collision-syscalls.ts` (cgame
//! collision traps from id Software `code/client/cl_cgame.c`,
//! `code/cgame/cg_public.h`, and `cg_syscalls.asm`). [`TraceRecord`] and
//! [`write_trace`] are local mirrors of `trace-record.ts` (owned by another
//! worker); sibling files reuse them via
//! `super::client_collision_syscalls::...`. [`BOX_MODEL_HANDLE`] mirrors
//! `SOURCE_BOX_MODEL_HANDLE` from the collision owner.

use qa_core::math::Vec3;

use super::client_state::{CallKind, HostCall, QvmRole, SyscallMemory};
use super::legacy_bot_abi::{
    CG_CM_BOXTRACE, CG_CM_CAPSULETRACE, CG_CM_INLINEMODEL, CG_CM_LOADMAP, CG_CM_NUMINLINEMODELS, CG_CM_POINTCONTENTS,
    CG_CM_TEMPBOXMODEL, CG_CM_TEMPCAPSULEMODEL, CG_CM_TRANSFORMEDBOXTRACE, CG_CM_TRANSFORMEDCAPSULETRACE,
    CG_CM_TRANSFORMEDPOINTCONTENTS,
};
use crate::error::GuestError;

/// Byte length of `trace_t`.
pub const QVM_TRACE_BYTES: usize = 56;
/// Temporary box-model handle: angles are ignored for this handle.
pub const BOX_MODEL_HANDLE: i32 = 255;

/// Collision trace shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceShape {
    /// Box trace.
    Box,
    /// Capsule trace.
    Capsule,
}

/// Trace query bounds plus mask.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Bounds minimum (zero when the guest passes null).
    pub mins: Vec3,
    /// Bounds maximum (zero when the guest passes null).
    pub maxs: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Contents mask.
    pub mask: i32,
}

/// Collision trace result record.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceRecord {
    /// Started inside solid.
    pub all_solid: bool,
    /// Start point is solid.
    pub start_solid: bool,
    /// Completed fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Impact plane normal.
    pub plane_normal: Vec3,
    /// Impact plane distance.
    pub plane_distance: f32,
    /// Plane type.
    pub plane_type: u8,
    /// Plane sign bits.
    pub plane_signbits: u8,
    /// Surface flags.
    pub surface_flags: i32,
    /// Contents.
    pub contents: i32,
    /// Hit entity number (cgame assigns entity identity later, so this is 0).
    pub entity_num: i32,
}

/// Write a `trace_t` record into guest memory.
pub fn write_trace(memory: &mut SyscallMemory, word: i32, value: &TraceRecord) -> Result<(), GuestError> {
    let range = memory.span(word, QVM_TRACE_BYTES, 0)?;
    memory.write_i32(range.start, i32::from(value.all_solid))?;
    memory.write_i32(range.start + 4, i32::from(value.start_solid))?;
    memory.write_f32(range.start + 8, value.fraction)?;
    memory.write_vec3(range.start + 12, &value.end)?;
    memory.write_vec3(range.start + 24, &value.plane_normal)?;
    memory.write_f32(range.start + 36, value.plane_distance)?;
    memory.set(range.start + 40, value.plane_type)?;
    memory.set(range.start + 41, value.plane_signbits)?;
    memory.write_u16(range.start + 42, 0)?;
    memory.write_i32(range.start + 44, value.surface_flags)?;
    memory.write_i32(range.start + 48, value.contents)?;
    memory.write_i32(range.start + 52, value.entity_num)?;
    Ok(())
}

/// Host collision-model surface used by the traps.
pub trait ClientCollisionHost {
    /// Load a collision map.
    fn load_map(&mut self, name: &str);
    /// Number of inline models.
    fn model_count(&mut self) -> i32;
    /// Inline model handle by index.
    fn inline_model(&mut self, index: i32) -> i32;
    /// Temporary box/capsule model handle.
    fn temp_box_model(&mut self, mins: Vec3, maxs: Vec3, capsule: bool) -> i32;
    /// Whether the world has BSP nodes.
    fn has_nodes(&mut self) -> bool;
    /// Point contents.
    fn point_contents(&mut self, point: Vec3, handle: i32) -> i32;
    /// Transformed point contents.
    fn transformed_point_contents(&mut self, point: Vec3, handle: i32, origin: Vec3, angles: Vec3) -> i32;
    /// Pre-BSP trace result for unloaded worlds, if the host short-circuits.
    fn trace_without_nodes(&mut self, handle: i32) -> Option<TraceRecord>;
    /// Trace against a model.
    fn trace(&mut self, query: &TraceQuery, handle: i32) -> TraceRecord;
    /// Transformed trace against a model.
    fn transformed_trace(&mut self, query: &TraceQuery, handle: i32, origin: Vec3, angles: Vec3) -> TraceRecord;
}

fn zero() -> Vec3 {
    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
}

/// Dispatch a client-collision trap. Returns `Ok(None)` when unhandled.
pub fn client_collision_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    services: &mut dyn ClientCollisionHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role != QvmRole::Cgame {
        return Ok(None);
    }
    if call.code == CG_CM_LOADMAP {
        let name = memory.read_string(call.int(1)?)?;
        services.load_map(&name);
        return Ok(Some(0));
    }
    if !matches!(
        call.code,
        CG_CM_NUMINLINEMODELS
            | CG_CM_INLINEMODEL
            | CG_CM_TEMPBOXMODEL
            | CG_CM_POINTCONTENTS
            | CG_CM_TRANSFORMEDPOINTCONTENTS
            | CG_CM_BOXTRACE
            | CG_CM_TRANSFORMEDBOXTRACE
            | CG_CM_TEMPCAPSULEMODEL
            | CG_CM_CAPSULETRACE
            | CG_CM_TRANSFORMEDCAPSULETRACE
    ) {
        return Ok(None);
    }
    match call.code {
        CG_CM_NUMINLINEMODELS => Ok(Some(services.model_count())),
        CG_CM_INLINEMODEL => Ok(Some(services.inline_model(call.int(1)?))),
        CG_CM_TEMPBOXMODEL | CG_CM_TEMPCAPSULEMODEL => {
            let mins = memory.read_vec3_ptr(call.int(1)?)?;
            let maxs = memory.read_vec3_ptr(call.int(2)?)?;
            Ok(Some(services.temp_box_model(
                mins,
                maxs,
                call.code == CG_CM_TEMPCAPSULEMODEL,
            )))
        }
        CG_CM_POINTCONTENTS => {
            if !services.has_nodes() {
                return Ok(Some(0));
            }
            Ok(Some(
                services.point_contents(memory.read_vec3_ptr(call.int(1)?)?, call.int(2)?),
            ))
        }
        CG_CM_TRANSFORMEDPOINTCONTENTS => {
            let point = memory.read_vec3_ptr(call.int(1)?)?;
            let handle = call.int(2)?;
            let origin = memory.read_vec3_ptr(call.int(3)?)?;
            let angles = if handle == BOX_MODEL_HANDLE {
                zero()
            } else {
                memory.read_vec3_ptr(call.int(4)?)?
            };
            Ok(Some(services.transformed_point_contents(point, handle, origin, angles)))
        }
        CG_CM_BOXTRACE | CG_CM_TRANSFORMEDBOXTRACE | CG_CM_CAPSULETRACE | CG_CM_TRANSFORMEDCAPSULETRACE => {
            let output = call.int(1)?;
            let handle = call.int(6)?;
            let mask = call.int(7)?;
            if call.code == CG_CM_BOXTRACE || call.code == CG_CM_CAPSULETRACE {
                if let Some(unloaded) = services.trace_without_nodes(handle) {
                    let mut record = unloaded;
                    record.entity_num = 0;
                    write_trace(memory, output, &record)?;
                    return Ok(Some(0));
                }
            }
            let start = memory.read_vec3_ptr(call.int(2)?)?;
            let end = memory.read_vec3_ptr(call.int(3)?)?;
            let mins_word = call.int(4)?;
            let maxs_word = call.int(5)?;
            let mins = if mins_word == 0 {
                zero()
            } else {
                memory.read_vec3_ptr(mins_word)?
            };
            let maxs = if maxs_word == 0 {
                zero()
            } else {
                memory.read_vec3_ptr(maxs_word)?
            };
            let query = TraceQuery {
                start,
                end,
                mins,
                maxs,
                shape: if call.code == CG_CM_CAPSULETRACE || call.code == CG_CM_TRANSFORMEDCAPSULETRACE {
                    TraceShape::Capsule
                } else {
                    TraceShape::Box
                },
                mask,
            };
            let mut result = if call.code == CG_CM_TRANSFORMEDBOXTRACE || call.code == CG_CM_TRANSFORMEDCAPSULETRACE {
                let origin = memory.read_vec3_ptr(call.int(8)?)?;
                let angles = if handle == BOX_MODEL_HANDLE {
                    zero()
                } else {
                    memory.read_vec3_ptr(call.int(9)?)?
                };
                services.transformed_trace(&query, handle, origin, angles)
            } else {
                services.trace(&query, handle)
            };
            result.entity_num = 0;
            write_trace(memory, output, &result)?;
            Ok(Some(0))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_state::AbiProfile;
    use super::*;

    struct FakeModels {
        log: Vec<String>,
        nodes: bool,
        short_circuit: bool,
    }

    fn record() -> TraceRecord {
        TraceRecord {
            all_solid: false,
            start_solid: true,
            fraction: 0.5,
            end: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            plane_normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            plane_distance: 4.0,
            plane_type: 2,
            plane_signbits: 1,
            surface_flags: 7,
            contents: 8,
            entity_num: 0,
        }
    }

    impl ClientCollisionHost for FakeModels {
        fn load_map(&mut self, name: &str) {
            self.log.push(format!("load {name}"));
        }
        fn model_count(&mut self) -> i32 {
            12
        }
        fn inline_model(&mut self, index: i32) -> i32 {
            100 + index
        }
        fn temp_box_model(&mut self, _mins: Vec3, _maxs: Vec3, capsule: bool) -> i32 {
            if capsule {
                256
            } else {
                255
            }
        }
        fn has_nodes(&mut self) -> bool {
            self.nodes
        }
        fn point_contents(&mut self, _point: Vec3, handle: i32) -> i32 {
            1000 + handle
        }
        fn transformed_point_contents(&mut self, _point: Vec3, handle: i32, _origin: Vec3, angles: Vec3) -> i32 {
            self.log.push(format!("tpc {handle} {}", angles.x));
            handle
        }
        fn trace_without_nodes(&mut self, _handle: i32) -> Option<TraceRecord> {
            self.short_circuit.then(record)
        }
        fn trace(&mut self, query: &TraceQuery, handle: i32) -> TraceRecord {
            self.log
                .push(format!("trace {handle} {:?} {}", query.shape, query.mask));
            record()
        }
        fn transformed_trace(&mut self, query: &TraceQuery, handle: i32, _origin: Vec3, _angles: Vec3) -> TraceRecord {
            self.log.push(format!("ttrace {handle} {:?}", query.shape));
            record()
        }
    }

    fn cg(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Cgame, code, args, AbiProfile::Modern)
    }

    #[test]
    fn loadmap_models_and_contents() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(512, "maps/q3dm1.bsp", 15).unwrap();
        memory.write_vec3(256, &Vec3 { x: 1.0, y: 1.0, z: 1.0 }).unwrap();
        memory
            .write_vec3(
                768,
                &Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -16.0,
                },
            )
            .unwrap();
        memory
            .write_vec3(
                1024,
                &Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 16.0,
                },
            )
            .unwrap();
        let mut models = FakeModels {
            log: Vec::new(),
            nodes: true,
            short_circuit: false,
        };
        assert_eq!(
            client_collision_syscall(&cg(CG_CM_LOADMAP, &[512]), &mut memory, &mut models).unwrap(),
            Some(0)
        );
        assert_eq!(
            client_collision_syscall(&cg(CG_CM_NUMINLINEMODELS, &[]), &mut memory, &mut models).unwrap(),
            Some(12)
        );
        assert_eq!(
            client_collision_syscall(&cg(CG_CM_INLINEMODEL, &[3]), &mut memory, &mut models).unwrap(),
            Some(103)
        );
        assert_eq!(
            client_collision_syscall(&cg(CG_CM_TEMPBOXMODEL, &[768, 1024]), &mut memory, &mut models).unwrap(),
            Some(255)
        );
        assert_eq!(
            client_collision_syscall(&cg(CG_CM_TEMPCAPSULEMODEL, &[768, 1024]), &mut memory, &mut models).unwrap(),
            Some(256)
        );
        assert_eq!(
            client_collision_syscall(&cg(CG_CM_POINTCONTENTS, &[256, 4]), &mut memory, &mut models).unwrap(),
            Some(1004)
        );
        models.nodes = false;
        assert_eq!(
            client_collision_syscall(&cg(CG_CM_POINTCONTENTS, &[256, 4]), &mut memory, &mut models).unwrap(),
            Some(0)
        );
        assert_eq!(models.log[0], "load maps/q3dm1.bsp".to_string());
    }

    #[test]
    fn transformed_contents_ignores_box_angles() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        for (index, word) in [256, 512, 768, 1024].iter().enumerate() {
            memory
                .write_vec3(
                    *word,
                    &Vec3 {
                        x: index as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                )
                .unwrap();
        }
        let mut models = FakeModels {
            log: Vec::new(),
            nodes: true,
            short_circuit: false,
        };
        assert_eq!(
            client_collision_syscall(
                &cg(CG_CM_TRANSFORMEDPOINTCONTENTS, &[256, 255, 768, 1024]),
                &mut memory,
                &mut models
            )
            .unwrap(),
            Some(255)
        );
        assert_eq!(models.log, vec!["tpc 255 0".to_string()]);
    }

    #[test]
    fn traces_write_record_with_zero_entity() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_vec3(256, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        memory
            .write_vec3(
                512,
                &Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -64.0,
                },
            )
            .unwrap();
        let mut models = FakeModels {
            log: Vec::new(),
            nodes: true,
            short_circuit: false,
        };
        let args = [1024, 256, 512, 0, 0, 7, 3];
        assert_eq!(
            client_collision_syscall(&cg(CG_CM_BOXTRACE, &args), &mut memory, &mut models).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_i32(1024 + 4).unwrap(), 1);
        assert_eq!(memory.read_f32(1024 + 8).unwrap(), 0.5);
        assert_eq!(memory.read_i32(1024 + 52).unwrap(), 0);
        assert_eq!(memory.get(1024 + 40).unwrap(), 2);
        let args = [1024, 256, 512, 0, 0, 7, 3, 256, 512];
        assert_eq!(
            client_collision_syscall(&cg(CG_CM_TRANSFORMEDCAPSULETRACE, &args), &mut memory, &mut models).unwrap(),
            Some(0)
        );
        assert_eq!(
            models.log,
            vec!["trace 7 Box 3".to_string(), "ttrace 7 Capsule".to_string()]
        );
    }

    #[test]
    fn unloaded_short_circuit() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_vec3(256, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        memory.write_vec3(512, &Vec3 { x: 0.0, y: 0.0, z: 1.0 }).unwrap();
        let mut models = FakeModels {
            log: Vec::new(),
            nodes: false,
            short_circuit: true,
        };
        assert_eq!(
            client_collision_syscall(
                &cg(CG_CM_BOXTRACE, &[1024, 256, 512, 0, 0, 7, 1]),
                &mut memory,
                &mut models
            )
            .unwrap(),
            Some(0)
        );
        assert!(models.log.is_empty());
        assert_eq!(memory.read_f32(1024 + 8).unwrap(), 0.5);
    }

    #[test]
    fn trace_record_layout() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        write_trace(&mut memory, 128, &record()).unwrap();
        assert_eq!(memory.read_i32(128).unwrap(), 0);
        assert_eq!(memory.read_i32(132).unwrap(), 1);
        assert_eq!(memory.read_f32(136).unwrap(), 0.5);
        assert_eq!(memory.read_vec3(140).unwrap(), Vec3 { x: 1.0, y: 2.0, z: 3.0 });
        assert_eq!(memory.read_f32(164).unwrap(), 4.0);
        assert_eq!(memory.read_u16(170).unwrap(), 0);
        assert_eq!(memory.read_i32(172).unwrap(), 7);
        assert_eq!(memory.read_i32(176).unwrap(), 8);
        assert!(write_trace(&mut memory, 4096 - 55, &record()).is_err());
    }

    #[test]
    fn routing() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut models = FakeModels {
            log: Vec::new(),
            nodes: true,
            short_circuit: false,
        };
        assert_eq!(
            client_collision_syscall(&cg(21, &[]), &mut memory, &mut models).unwrap(),
            None
        );
        let other = HostCall::engine(QvmRole::Ui, CG_CM_LOADMAP, &[512], AbiProfile::Modern);
        assert_eq!(
            client_collision_syscall(&other, &mut memory, &mut models).unwrap(),
            None
        );
    }
}
