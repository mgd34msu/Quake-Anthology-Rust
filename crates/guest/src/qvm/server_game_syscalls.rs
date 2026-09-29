//! Server game-module traps: spatial queries plus client/information traps.
//!
//! Provenance: `src/compat/qvm/server-game-syscalls.ts` (Q3
//! `server/sv_game.c` guest ABI). Client and information traps delegate to
//! [`super::client_game_syscalls`] and [`super::server_info_syscalls`] via
//! forwarding adapters, matching the donor's delegation; the entity-token
//! trap is implemented inline from the donor's `entity-tokens.ts`
//! consumption rule (consume a token before copying, retain a final
//! nonempty token at EOF).

use qa_core::math::Vec3;

use super::client_collision_syscalls::{TraceRecord, TraceShape};
use super::client_game_syscalls::{ClientGameHost, client_game_syscall};
use super::client_state::{AbiProfile, CallKind, HostCall, QvmRole, SyscallMemory, WireUserCommand};
use super::legacy_bot_abi::{
    G_ADJUST_AREA_PORTAL_STATE, G_AREAS_CONNECTED, G_DROP_CLIENT, G_ENTITIES_IN_BOX, G_ENTITY_CONTACT,
    G_ENTITY_CONTACTCAPSULE, G_GET_CONFIGSTRING, G_GET_ENTITY_TOKEN, G_GET_SERVERINFO, G_GET_USERCMD,
    G_GET_USERINFO, G_IN_PVS, G_IN_PVS_IGNORE_PORTALS, G_LINKENTITY, G_POINT_CONTENTS, G_SEND_SERVER_COMMAND,
    G_SET_BRUSH_MODEL, G_SET_CONFIGSTRING, G_SET_USERINFO, G_TRACE, G_TRACECAPSULE, G_UNLINKENTITY,
};
use super::server_info_syscalls::{ServerInformationHost, server_information_syscall};
use crate::error::GuestError;

/// Server trace query.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerTraceQuery {
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
    /// Entity number the trace passes through.
    pub pass_entity_num: i32,
    /// Contents mask.
    pub mask: i32,
}

/// World bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Minimum corner.
    pub min: Vec3,
    /// Maximum corner.
    pub max: Vec3,
}

/// Host spatial-operation surface used by the traps.
pub trait ServerSpatialHost {
    /// Trace against the world.
    fn trace(&mut self, query: &ServerTraceQuery) -> TraceRecord;
    /// Point contents.
    fn point_contents(&mut self, point: Vec3, pass_entity_num: i32) -> i32;
    /// Entity numbers touching bounds (at most `maximum`, when nonnegative).
    fn area_entities(&mut self, bounds: Bounds, maximum: i32) -> Vec<i32>;
    /// Entity contact test.
    fn entity_contact(&mut self, bounds: Bounds, slot: i32, capsule: bool) -> bool;
    /// Set an entity's brush model.
    fn set_brush_model(&mut self, slot: i32, name: &str);
    /// Adjust an area-portal state.
    fn adjust_area_portal_state(&mut self, slot: i32, open: bool);
    /// PVS visibility between two points.
    fn in_pvs(&mut self, first: Vec3, second: Vec3, ignore_portals: bool) -> bool;
    /// Area connectivity.
    fn areas_connected(&mut self, first: i32, second: i32) -> bool;
    /// Link an entity.
    fn link(&mut self, slot: i32);
    /// Unlink an entity.
    fn unlink(&mut self, slot: i32);
}

/// Host server-game surface: client table, configstrings, entities, space.
pub trait ServerGameHost: ServerSpatialHost {
    /// Selected ABI profile.
    fn abi_profile(&self) -> AbiProfile;
    /// Maximum client count.
    fn max_clients(&self) -> i32;
    /// Entity slot for a guest entity pointer.
    fn number_from_pointer(&mut self, word: i32) -> i32;
    /// Userinfo string for a slot.
    fn get_userinfo(&mut self, slot: i32) -> String;
    /// Set the userinfo string for a slot.
    fn set_userinfo(&mut self, slot: i32, value: &str);
    /// Current user command for a slot.
    fn get_user_command(&mut self, slot: i32) -> WireUserCommand;
    /// Drop a client with a reason.
    fn drop_client(&mut self, slot: i32, reason: &str);
    /// Send a server command to a slot (`-1` broadcasts).
    fn send_server_command(&mut self, slot: i32, text: &str);
    /// Configstring value at a canonical index.
    fn config_get(&mut self, index: i32) -> String;
    /// Set a configstring at a canonical index.
    fn config_set(&mut self, index: i32, value: &str);
    /// Server-info string.
    fn server_info(&mut self) -> String;
    /// Next entity token plus whether parsing ended.
    fn entity_token(&mut self) -> (String, bool);
}

struct ClientAdapter<'a, H: ?Sized> {
    host: &'a mut H,
}

impl<H: ServerGameHost + ?Sized> ClientGameHost for ClientAdapter<'_, H> {
    fn abi_profile(&self) -> AbiProfile {
        self.host.abi_profile()
    }
    fn max_clients(&self) -> i32 {
        self.host.max_clients()
    }
    fn get_userinfo(&mut self, slot: i32) -> String {
        self.host.get_userinfo(slot)
    }
    fn set_userinfo(&mut self, slot: i32, value: &str) {
        self.host.set_userinfo(slot, value);
    }
    fn get_user_command(&mut self, slot: i32) -> WireUserCommand {
        self.host.get_user_command(slot)
    }
    fn drop_client(&mut self, slot: i32, reason: &str) {
        self.host.drop_client(slot, reason);
    }
    fn send_server_command(&mut self, slot: i32, text: &str) {
        self.host.send_server_command(slot, text);
    }
}

impl<H: ServerGameHost + ?Sized> ServerInformationHost for ClientAdapter<'_, H> {
    fn abi_profile(&self) -> AbiProfile {
        self.host.abi_profile()
    }
    fn config_get(&mut self, index: i32) -> String {
        self.host.config_get(index)
    }
    fn config_set(&mut self, index: i32, value: &str) {
        self.host.config_set(index, value);
    }
    fn server_info(&mut self) -> String {
        self.host.server_info()
    }
}

/// Dispatch a server game-module trap. Returns `Ok(None)` when unhandled.
pub fn server_game_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    services: &mut dyn ServerGameHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role != QvmRole::Qagame {
        return Ok(None);
    }
    match call.code {
        G_DROP_CLIENT | G_SEND_SERVER_COMMAND | G_GET_USERINFO | G_SET_USERINFO | G_GET_USERCMD => {
            let mut adapter = ClientAdapter { host: services };
            client_game_syscall(call, memory, &mut adapter)
        }
        G_SET_CONFIGSTRING | G_GET_CONFIGSTRING | G_GET_SERVERINFO => {
            let mut adapter = ClientAdapter { host: services };
            server_information_syscall(call, memory, &mut adapter)
        }
        G_SET_BRUSH_MODEL => {
            if call.int(2)? == 0 {
                return Err(GuestError::runtime("SV_SetBrushModel: NULL"));
            }
            let name = memory.read_string(call.int(2)?)?;
            if !name.starts_with('*') {
                return Err(GuestError::runtime(format!("SV_SetBrushModel: {name} isn't a brush model")));
            }
            let slot = services.number_from_pointer(call.int(1)?);
            services.set_brush_model(slot, &name);
            Ok(Some(0))
        }
        G_TRACE | G_TRACECAPSULE => {
            let zero = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
            let mins_word = call.int(3)?;
            let maxs_word = call.int(4)?;
            let query = ServerTraceQuery {
                start: memory.read_vec3_ptr(call.int(2)?)?,
                end: memory.read_vec3_ptr(call.int(5)?)?,
                mins: if mins_word == 0 { zero } else { memory.read_vec3_ptr(mins_word)? },
                maxs: if maxs_word == 0 { zero } else { memory.read_vec3_ptr(maxs_word)? },
                shape: if call.code == G_TRACECAPSULE { TraceShape::Capsule } else { TraceShape::Box },
                pass_entity_num: call.int(6)?,
                mask: call.int(7)?,
            };
            let result = services.trace(&query);
            super::client_collision_syscalls::write_trace(memory, call.int(1)?, &result)?;
            Ok(Some(0))
        }
        G_POINT_CONTENTS => {
            Ok(Some(services.point_contents(memory.read_vec3_ptr(call.int(1)?)?, call.int(2)?)))
        }
        G_IN_PVS | G_IN_PVS_IGNORE_PORTALS => {
            let first = memory.read_vec3_ptr(call.int(1)?)?;
            let second = memory.read_vec3_ptr(call.int(2)?)?;
            Ok(Some(i32::from(services.in_pvs(first, second, call.code == G_IN_PVS_IGNORE_PORTALS))))
        }
        G_ADJUST_AREA_PORTAL_STATE => {
            let slot = services.number_from_pointer(call.int(1)?);
            services.adjust_area_portal_state(slot, call.int(2)? != 0);
            Ok(Some(0))
        }
        G_AREAS_CONNECTED => Ok(Some(i32::from(services.areas_connected(call.int(1)?, call.int(2)?)))),
        G_LINKENTITY => {
            let slot = services.number_from_pointer(call.int(1)?);
            services.link(slot);
            Ok(Some(0))
        }
        G_UNLINKENTITY => {
            let slot = services.number_from_pointer(call.int(1)?);
            services.unlink(slot);
            Ok(Some(0))
        }
        G_ENTITIES_IN_BOX => {
            let bounds = Bounds {
                min: memory.read_vec3_ptr(call.int(1)?)?,
                max: memory.read_vec3_ptr(call.int(2)?)?,
            };
            let maximum = call.int(4)?;
            let result = services.area_entities(bounds, maximum);
            if maximum >= 0 && result.len() as i32 > maximum {
                return Err(GuestError::invalid("Area entity owner exceeded output capacity"));
            }
            if !result.is_empty() {
                let word = call.int(3)?;
                memory.span(word, result.len() * 4, 0)?;
                let base = memory.pointer(word).expect("checked span");
                for (index, number) in result.iter().enumerate() {
                    memory.write_i32(base + index * 4, *number)?;
                }
            }
            Ok(Some(result.len() as i32))
        }
        G_ENTITY_CONTACT | G_ENTITY_CONTACTCAPSULE => {
            let bounds = Bounds {
                min: memory.read_vec3_ptr(call.int(1)?)?,
                max: memory.read_vec3_ptr(call.int(2)?)?,
            };
            let slot = services.number_from_pointer(call.int(3)?);
            Ok(Some(i32::from(services.entity_contact(bounds, slot, call.code == G_ENTITY_CONTACTCAPSULE))))
        }
        G_GET_ENTITY_TOKEN => {
            let (token, ended) = services.entity_token();
            memory.write_string(call.int(1)?, &token, call.int(2)? as usize)?;
            Ok(Some(i32::from(!ended || !token.is_empty())))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_collision_syscalls::TraceRecord;
    use super::*;

    struct FakeServer {
        log: Vec<String>,
        tokens: Vec<(String, bool)>,
    }

    fn record() -> TraceRecord {
        TraceRecord {
            all_solid: false,
            start_solid: false,
            fraction: 1.0,
            end: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            plane_normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            plane_distance: 0.0,
            plane_type: 0,
            plane_signbits: 0,
            surface_flags: 0,
            contents: 0,
            entity_num: 9,
        }
    }

    impl ServerSpatialHost for FakeServer {
        fn trace(&mut self, query: &ServerTraceQuery) -> TraceRecord {
            self.log.push(format!("trace {:?} {} {}", query.shape, query.pass_entity_num, query.mask));
            record()
        }
        fn point_contents(&mut self, _point: Vec3, pass: i32) -> i32 {
            100 + pass
        }
        fn area_entities(&mut self, _bounds: Bounds, _maximum: i32) -> Vec<i32> {
            vec![1, 2, 3]
        }
        fn entity_contact(&mut self, _bounds: Bounds, slot: i32, capsule: bool) -> bool {
            self.log.push(format!("contact {slot} {capsule}"));
            true
        }
        fn set_brush_model(&mut self, slot: i32, name: &str) {
            self.log.push(format!("brush {slot} {name}"));
        }
        fn adjust_area_portal_state(&mut self, slot: i32, open: bool) {
            self.log.push(format!("portal {slot} {open}"));
        }
        fn in_pvs(&mut self, _first: Vec3, _second: Vec3, ignore: bool) -> bool {
            !ignore
        }
        fn areas_connected(&mut self, first: i32, second: i32) -> bool {
            first == second
        }
        fn link(&mut self, slot: i32) {
            self.log.push(format!("link {slot}"));
        }
        fn unlink(&mut self, slot: i32) {
            self.log.push(format!("unlink {slot}"));
        }
    }

    impl ServerGameHost for FakeServer {
        fn abi_profile(&self) -> AbiProfile {
            AbiProfile::Modern
        }
        fn max_clients(&self) -> i32 {
            2
        }
        fn number_from_pointer(&mut self, word: i32) -> i32 {
            word / 16
        }
        fn get_userinfo(&mut self, _slot: i32) -> String {
            "\\name\\x".to_string()
        }
        fn set_userinfo(&mut self, slot: i32, value: &str) {
            self.log.push(format!("userinfo {slot} {value}"));
        }
        fn get_user_command(&mut self, _slot: i32) -> WireUserCommand {
            WireUserCommand::default()
        }
        fn drop_client(&mut self, slot: i32, reason: &str) {
            self.log.push(format!("drop {slot} {reason}"));
        }
        fn send_server_command(&mut self, slot: i32, text: &str) {
            self.log.push(format!("cmd {slot} {text}"));
        }
        fn config_get(&mut self, _index: i32) -> String {
            "cfg".to_string()
        }
        fn config_set(&mut self, index: i32, value: &str) {
            self.log.push(format!("cfg {index} {value}"));
        }
        fn server_info(&mut self) -> String {
            "info".to_string()
        }
        fn entity_token(&mut self) -> (String, bool) {
            if self.tokens.is_empty() {
                (String::new(), true)
            } else {
                self.tokens.remove(0)
            }
        }
    }

    fn game(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Qagame, code, args, AbiProfile::Modern)
    }

    fn server() -> FakeServer {
        FakeServer { log: Vec::new(), tokens: vec![("{".to_string(), false), (String::new(), true)] }
    }

    #[test]
    fn delegates_client_and_information_traps() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_string(512, "hi", 3).unwrap();
        let mut services = server();
        assert_eq!(server_game_syscall(&game(G_GET_USERINFO, &[0, 256, 64]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(memory.read_string(256).unwrap(), "\\name\\x");
        assert_eq!(server_game_syscall(&game(G_SEND_SERVER_COMMAND, &[-1, 512]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(server_game_syscall(&game(G_GET_CONFIGSTRING, &[3, 256, 64]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(memory.read_string(256).unwrap(), "cfg");
        assert_eq!(server_game_syscall(&game(G_GET_SERVERINFO, &[256, 64]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(services.log[0], "cmd -1 hi".to_string());
    }

    #[test]
    fn brush_model_validates_name() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_string(512, "*4", 3).unwrap();
        memory.write_string(1024, "bad", 4).unwrap();
        let mut services = server();
        assert_eq!(server_game_syscall(&game(G_SET_BRUSH_MODEL, &[32, 512]), &mut memory, &mut services).unwrap(), Some(0));
        assert!(server_game_syscall(&game(G_SET_BRUSH_MODEL, &[32, 1024]), &mut memory, &mut services).is_err());
        assert!(server_game_syscall(&game(G_SET_BRUSH_MODEL, &[32, 0]), &mut memory, &mut services).is_err());
        assert_eq!(services.log[0], "brush 2 *4".to_string());
    }

    #[test]
    fn trace_and_contents() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_vec3(256, &Vec3 { x: 0.0, y: 0.0, z: 10.0 }).unwrap();
        memory.write_vec3(512, &Vec3 { x: 0.0, y: 0.0, z: -10.0 }).unwrap();
        let mut services = server();
        assert_eq!(
            server_game_syscall(&game(G_TRACE, &[1024, 256, 0, 0, 512, 1, 3]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_f32(1032).unwrap(), 1.0);
        assert_eq!(memory.read_i32(1076).unwrap(), 9);
        assert_eq!(
            server_game_syscall(&game(G_TRACECAPSULE, &[1024, 256, 0, 0, 512, 1, 3]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(server_game_syscall(&game(G_POINT_CONTENTS, &[256, 2]), &mut memory, &mut services).unwrap(), Some(102));
        assert_eq!(services.log[0], "trace Box 1 3".to_string());
        assert_eq!(services.log[1], "trace Capsule 1 3".to_string());
    }

    #[test]
    fn pvs_portals_links_and_areas() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_vec3(256, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        memory.write_vec3(512, &Vec3 { x: 1.0, y: 1.0, z: 1.0 }).unwrap();
        let mut services = server();
        assert_eq!(server_game_syscall(&game(G_IN_PVS, &[256, 512]), &mut memory, &mut services).unwrap(), Some(1));
        assert_eq!(server_game_syscall(&game(G_IN_PVS_IGNORE_PORTALS, &[256, 512]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(server_game_syscall(&game(G_AREAS_CONNECTED, &[3, 3]), &mut memory, &mut services).unwrap(), Some(1));
        assert_eq!(server_game_syscall(&game(G_ADJUST_AREA_PORTAL_STATE, &[16, 1]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(server_game_syscall(&game(G_LINKENTITY, &[32]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(server_game_syscall(&game(G_UNLINKENTITY, &[32]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(
            services.log,
            vec!["portal 1 true".to_string(), "link 2".to_string(), "unlink 2".to_string()]
        );
    }

    #[test]
    fn entities_in_box_and_contact() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        memory.write_vec3(256, &Vec3 { x: -8.0, y: -8.0, z: -8.0 }).unwrap();
        memory.write_vec3(512, &Vec3 { x: 8.0, y: 8.0, z: 8.0 }).unwrap();
        let mut services = server();
        assert_eq!(
            server_game_syscall(&game(G_ENTITIES_IN_BOX, &[256, 512, 1024, 8]), &mut memory, &mut services).unwrap(),
            Some(3)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 1);
        assert_eq!(memory.read_i32(1032).unwrap(), 3);
        assert!(server_game_syscall(&game(G_ENTITIES_IN_BOX, &[256, 512, 1024, 2]), &mut memory, &mut services).is_err());
        assert_eq!(server_game_syscall(&game(G_ENTITY_CONTACT, &[256, 512, 48]), &mut memory, &mut services).unwrap(), Some(1));
        assert_eq!(
            server_game_syscall(&game(G_ENTITY_CONTACTCAPSULE, &[256, 512, 48]), &mut memory, &mut services).unwrap(),
            Some(1)
        );
        assert_eq!(services.log, vec!["contact 3 false".to_string(), "contact 3 true".to_string()]);
    }

    #[test]
    fn entity_token_consumes_before_copying() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        let mut services = server();
        assert_eq!(server_game_syscall(&game(G_GET_ENTITY_TOKEN, &[256, 64]), &mut memory, &mut services).unwrap(), Some(1));
        assert_eq!(memory.read_string(256).unwrap(), "{");
        assert_eq!(server_game_syscall(&game(G_GET_ENTITY_TOKEN, &[256, 64]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(server_game_syscall(&game(999, &[]), &mut memory, &mut services).unwrap(), None);
    }
}
