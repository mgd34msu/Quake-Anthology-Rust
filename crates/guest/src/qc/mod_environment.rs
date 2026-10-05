//! QuakeC mod environment: per-guest cvars and client visibility.
//!
//! Ported from `src/compat/qc/mod-environment.ts`.
//!
//! Local mirrors (owned by other workers' modules, noted here):
//! `QcModCvars` mirrors the `CvarRegistry` surface from
//! `src/core/cvars/index.ts`; `QcClientVisibility` mirrors
//! `Q1ClientVisibility` from `src/world/gameplay/q1-client-visibility.ts`;
//! `QcEnvMachine`/`QcEnvVm` mirror the `QcMachine` surfaces from
//! `src/compat/qc/machine.ts`; `QcEnvServices` mirrors the engine surface of
//! `ModHostServices` from `src/world/session/mods.ts`.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_core::numeric::native_atof;
use qa_core::time::SourceTime;

use super::mod_provider::{ModCallbackDeclaration, QcApiKind, QcProgramView, QcValueType};
use crate::error::GuestError;

/// Per-guest console variables.
#[derive(Debug, Default)]
pub struct QcModCvars {
    values: HashMap<String, String>,
}

impl QcModCvars {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a variable with its default.
    pub fn register(&mut self, name: &str, value: &str) -> Result<(), GuestError> {
        if name.is_empty() || name.contains('\0') {
            return Err(GuestError::invalid(format!("Invalid mod cvar {name}")));
        }
        self.values.entry(name.to_string()).or_insert_with(|| value.to_string());
        Ok(())
    }

    /// Assign a registered variable.
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), GuestError> {
        if !self.values.contains_key(name) {
            return Err(GuestError::invalid(format!("Unknown mod cvar {name}")));
        }
        self.values.insert(name.to_string(), value.to_string());
        Ok(())
    }

    /// Assign a variable, registering when absent.
    pub fn set_default(&mut self, name: &str, value: &str) -> Result<(), GuestError> {
        if self.values.contains_key(name) {
            self.set(name, value)
        } else {
            self.register(name, value)
        }
    }

    /// Find a variable value.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// Variable value as a number.
    #[must_use]
    pub fn variable_value(&self, name: &str) -> f64 {
        self.find(name).map(native_atof).unwrap_or(0.0)
    }

    /// Variable value as text.
    #[must_use]
    pub fn variable_string(&self, name: &str) -> String {
        self.find(name).unwrap_or("").to_string()
    }

    /// Capture the table.
    #[must_use]
    pub fn capture(&self) -> Vec<(String, String)> {
        let mut state: Vec<(String, String)> = self
            .values
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        state.sort_by(|left, right| left.0.cmp(&right.0));
        state
    }

    /// Restore the table.
    pub fn restore(&mut self, state: &[(String, String)]) {
        self.values.clear();
        for (name, value) in state {
            self.values.insert(name.clone(), value.clone());
        }
    }
}

/// Game mode for default cvars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum QcEnvMode {
    /// Single player.
    #[default]
    Single,
    /// Cooperative.
    Coop,
    /// Deathmatch.
    Deathmatch,
}

/// Engine environment configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QcEnvConfig {
    /// Skill level.
    pub skill: i32,
    /// Maximum clients.
    pub max_clients: u32,
    /// Game mode.
    pub mode: QcEnvMode,
    /// Gravity.
    pub gravity: f64,
}

impl Default for QcEnvConfig {
    fn default() -> Self {
        Self {
            skill: 1,
            max_clients: 1,
            mode: QcEnvMode::Single,
            gravity: 800.0,
        }
    }
}

/// Visible client description.
#[derive(Debug, Clone, PartialEq)]
pub struct QcVisibilityClient {
    /// Client actor.
    pub actor: ActorId,
    /// View offset.
    pub view_offset: Vec3,
    /// Notarget flag.
    pub notarget: bool,
}

/// Destination client-visibility queries.
pub trait QcClientVisibility {
    /// Find the visible client for an observer.
    fn check(&mut self, origin: Vec3, view_offset: Vec3, time: f64) -> Result<Option<ActorId>, GuestError>;
    /// Capture visibility state.
    fn capture(&self) -> Vec<u8>;
    /// Restore visibility state.
    fn restore(&mut self, state: &[u8]) -> Result<(), GuestError>;
}

/// Engine services for the environment.
pub trait QcEnvServices {
    /// Environment configuration.
    fn environment(&self) -> QcEnvConfig {
        QcEnvConfig::default()
    }
    /// Print engine text.
    fn print(&mut self, _text: &str) {}
    /// Current source time.
    fn now(&self) -> SourceTime;
    /// Presentation map name.
    fn presentation_map(&self) -> Option<String> {
        None
    }
    /// Whether destination client semantics exist.
    fn has_client_semantics(&self) -> bool {
        false
    }
    /// Visibility client for an actor.
    fn visibility_client(&self, _actor: &ActorId) -> Option<QcVisibilityClient> {
        None
    }
    /// Project an actor into a source reference.
    fn reference(&mut self, actor: Option<&ActorId>) -> i32;
}

/// Machine surface for global initialization.
pub trait QcEnvMachine {
    /// Program metadata view.
    fn program(&self) -> &dyn QcProgramView;
    /// Write a global float.
    fn set_global_float(&mut self, name: &str, value: f32) -> Result<(), GuestError>;
    /// Write a global integer.
    fn set_global_int(&mut self, name: &str, value: i32) -> Result<(), GuestError>;
    /// Allocate a managed string.
    fn strings_allocate(&mut self, text: &str) -> Result<i32, GuestError>;
}

/// VM surface for host builtins.
pub trait QcEnvVm {
    /// String argument.
    fn arg_string(&self, index: usize) -> Result<String, GuestError>;
    /// Concatenated string arguments from an index.
    fn var_string(&self, index: usize) -> String;
    /// Return a float.
    fn return_float(&mut self, value: f64);
    /// Return an integer.
    fn return_int(&mut self, value: i32);
    /// Global integer.
    fn global_int(&self, name: &str) -> Result<i32, GuestError>;
    /// Global float.
    fn global_float(&self, name: &str) -> Result<f64, GuestError>;
    /// Entity vector field.
    fn entity_vector(&self, reference: i32, field: &str) -> Result<Vec3, GuestError>;
}

/// Host builtin name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcEnvBuiltin {
    /// Read a cvar.
    Cvar,
    /// Assign a cvar.
    CvarSet,
    /// Developer print.
    Dprint,
    /// Find a visible client.
    Checkclient,
}

impl QcEnvBuiltin {
    /// Builtin source name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cvar => "cvar",
            Self::CvarSet => "cvar_set",
            Self::Dprint => "dprint",
            Self::Checkclient => "checkclient",
        }
    }

    /// Parse a builtin source name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "cvar" => Some(Self::Cvar),
            "cvar_set" => Some(Self::CvarSet),
            "dprint" => Some(Self::Dprint),
            "checkclient" => Some(Self::Checkclient),
            _ => None,
        }
    }
}

/// Saved environment state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcEnvCheckpoint {
    /// Cvar table.
    pub cvars: Vec<(String, String)>,
    /// Visibility state.
    pub visibility: Option<Vec<u8>>,
}

/// Per-guest environment.
pub struct QcModEnvironment<S, V> {
    services: S,
    cvars: QcModCvars,
    visibility: Option<V>,
}

impl<S: QcEnvServices, V: QcClientVisibility> QcModEnvironment<S, V> {
    /// Build with engine defaults and declared cvars.
    pub fn new(
        services: S,
        visibility: Option<V>,
        program: &dyn QcProgramView,
        declaration: &ModCallbackDeclaration,
    ) -> Result<Self, GuestError> {
        let environment = services.environment();
        let mut cvars = QcModCvars::new();
        let max_clients = declaration
            .clients
            .as_ref()
            .map_or(environment.max_clients as i32, |clients| clients.maximum);
        let defaults = [
            ("skill", environment.skill.to_string()),
            ("maxclients", max_clients.to_string()),
            (
                "coop",
                if environment.mode == QcEnvMode::Coop { "1" } else { "0" }.to_string(),
            ),
            (
                "deathmatch",
                if environment.mode == QcEnvMode::Deathmatch {
                    "1"
                } else {
                    "0"
                }
                .to_string(),
            ),
            ("teamplay", "0".to_string()),
            ("sv_gravity", environment.gravity.to_string()),
            (
                "sv_aim",
                if program.api_kind() == QcApiKind::Q1Quakeworld {
                    "2"
                } else {
                    "0.93"
                }
                .to_string(),
            ),
            ("sv_maxspeed", "320".to_string()),
            ("registered", "1".to_string()),
            ("developer", "0".to_string()),
            ("sv_cheats", "0".to_string()),
            ("samelevel", "0".to_string()),
            ("timelimit", "0".to_string()),
            ("fraglimit", "0".to_string()),
            ("gamecfg", "0".to_string()),
        ];
        for (name, value) in defaults {
            cvars.register(name, &value)?;
        }
        if program.api_kind() == QcApiKind::Q1Quakeworld {
            cvars.register("sv_phs", "1")?;
        }
        for variable in &declaration.cvars {
            cvars.set_default(&variable.name, &variable.value)?;
        }
        Ok(Self {
            services,
            cvars,
            visibility,
        })
    }

    /// Borrow the cvars.
    #[must_use]
    pub fn cvars(&self) -> &QcModCvars {
        &self.cvars
    }

    /// Borrow the cvars mutably.
    pub fn cvars_mut(&mut self) -> &mut QcModCvars {
        &mut self.cvars
    }

    /// Borrow the services.
    #[must_use]
    pub fn services(&self) -> &S {
        &self.services
    }

    /// Borrow the services mutably.
    pub fn services_mut(&mut self) -> &mut S {
        &mut self.services
    }

    /// Whether visibility queries exist.
    #[must_use]
    pub fn has_visibility(&self) -> bool {
        self.visibility.is_some()
    }

    /// Dispatch a host builtin.
    pub fn call_host(&mut self, builtin: QcEnvBuiltin, vm: &mut dyn QcEnvVm) -> Result<(), GuestError> {
        match builtin {
            QcEnvBuiltin::Cvar => {
                let name = vm.arg_string(0)?;
                vm.return_float(self.cvars.variable_value(&name));
                Ok(())
            }
            QcEnvBuiltin::CvarSet => {
                let name = vm.arg_string(0)?;
                let value = vm.arg_string(1)?;
                self.cvars.set(&name, &value)
            }
            QcEnvBuiltin::Dprint => {
                if self.cvars.variable_value("developer") != 0.0 {
                    let text = vm.var_string(0);
                    self.services.print(&text);
                }
                Ok(())
            }
            QcEnvBuiltin::Checkclient => {
                let visibility = self
                    .visibility
                    .as_mut()
                    .ok_or_else(|| GuestError::invalid("Mod checkclient requires destination client visibility"))?;
                let reference = vm.global_int("self")?;
                let origin = vm.entity_vector(reference, "origin")?;
                let view_offset = vm.entity_vector(reference, "view_ofs")?;
                let time = vm.global_float("time")?;
                let actor = visibility.check(origin, view_offset, time)?;
                let index = self.services.reference(actor.as_ref());
                vm.return_int(index);
                Ok(())
            }
        }
    }

    /// Initialize source globals from the environment.
    pub fn initialize_globals(&mut self, machine: &mut dyn QcEnvMachine) -> Result<(), GuestError> {
        for name in ["skill", "deathmatch", "coop", "teamplay"] {
            if machine.program().global_type(name) == Some(QcValueType::Float) {
                machine.set_global_float(name, self.cvars.variable_value(name) as f32)?;
            }
        }
        if machine.program().global_type("time") == Some(QcValueType::Float) {
            machine.set_global_float("time", self.services.now().as_seconds_f64() as f32)?;
        }
        if machine.program().global_type("mapname") == Some(QcValueType::String) {
            if let Some(map) = self.services.presentation_map() {
                let short = map.strip_prefix("maps/").unwrap_or(map.as_str());
                let short = short.strip_suffix(".bsp").unwrap_or(short);
                let index = machine.strings_allocate(short)?;
                machine.set_global_int("mapname", index)?;
            }
        }
        Ok(())
    }

    /// Visibility client for an actor.
    pub fn client(&self, actor: &ActorId) -> Result<Option<QcVisibilityClient>, GuestError> {
        if !self.services.has_client_semantics() {
            return Err(GuestError::invalid(
                "Mod client field requires destination client semantics",
            ));
        }
        Ok(self.services.visibility_client(actor))
    }

    /// Capture environment state.
    pub fn capture(&self) -> QcEnvCheckpoint {
        QcEnvCheckpoint {
            cvars: self.cvars.capture(),
            visibility: self.visibility.as_ref().map(QcClientVisibility::capture),
        }
    }

    /// Restore environment state.
    pub fn restore(&mut self, saved: &QcEnvCheckpoint) -> Result<(), GuestError> {
        self.cvars.restore(&saved.cvars);
        match (self.visibility.as_mut(), saved.visibility.as_deref()) {
            (Some(visibility), Some(state)) => visibility.restore(state),
            (None, None) => Ok(()),
            _ => Err(GuestError::BadSave("Saved mod client visibility differs".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::mod_provider::{ModClientDeclaration, ModCvar, ModProgramRef, QcApiKind, QcFunctionView};

    struct FakeProgram {
        globals: HashMap<String, QcValueType>,
    }

    impl QcProgramView for FakeProgram {
        fn digest(&self) -> &str {
            "abc"
        }

        fn api_kind(&self) -> QcApiKind {
            QcApiKind::Q1Netquake
        }

        fn field_type(&self, _name: &str) -> Option<QcValueType> {
            None
        }

        fn global_type(&self, name: &str) -> Option<QcValueType> {
            self.globals.get(name).copied()
        }

        fn function_named(&self, _name: &str) -> Option<QcFunctionView> {
            None
        }

        fn function_at(&self, _index: i32) -> Option<QcFunctionView> {
            None
        }

        fn functions(&self) -> Vec<QcFunctionView> {
            Vec::new()
        }
    }

    struct FakeServices {
        printed: Vec<String>,
        clients: HashMap<ActorId, QcVisibilityClient>,
        references: Vec<Option<ActorId>>,
    }

    impl QcEnvServices for FakeServices {
        fn now(&self) -> SourceTime {
            SourceTime::Seconds(4.0)
        }

        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }

        fn presentation_map(&self) -> Option<String> {
            Some("maps/e1m1.bsp".to_string())
        }

        fn has_client_semantics(&self) -> bool {
            true
        }

        fn visibility_client(&self, actor: &ActorId) -> Option<QcVisibilityClient> {
            self.clients.get(actor).cloned()
        }

        fn reference(&mut self, actor: Option<&ActorId>) -> i32 {
            self.references.push(actor.cloned());
            actor.map_or(0, |actor| actor.slot() as i32)
        }
    }

    struct FakeVisibility {
        next: Option<ActorId>,
        checks: usize,
        state: u8,
    }

    impl QcClientVisibility for FakeVisibility {
        fn check(&mut self, _origin: Vec3, _view_offset: Vec3, _time: f64) -> Result<Option<ActorId>, GuestError> {
            self.checks += 1;
            Ok(self.next.clone())
        }

        fn capture(&self) -> Vec<u8> {
            vec![self.state]
        }

        fn restore(&mut self, state: &[u8]) -> Result<(), GuestError> {
            self.state = state
                .first()
                .copied()
                .ok_or_else(|| GuestError::BadSave("Empty visibility state".to_string()))?;
            Ok(())
        }
    }

    struct FakeMachine {
        program: FakeProgram,
        floats: HashMap<String, f32>,
        ints: HashMap<String, i32>,
        strings: Vec<String>,
    }

    impl QcEnvMachine for FakeMachine {
        fn program(&self) -> &dyn QcProgramView {
            &self.program
        }

        fn set_global_float(&mut self, name: &str, value: f32) -> Result<(), GuestError> {
            self.floats.insert(name.to_string(), value);
            Ok(())
        }

        fn set_global_int(&mut self, name: &str, value: i32) -> Result<(), GuestError> {
            self.ints.insert(name.to_string(), value);
            Ok(())
        }

        fn strings_allocate(&mut self, text: &str) -> Result<i32, GuestError> {
            self.strings.push(text.to_string());
            Ok(self.strings.len() as i32 - 1)
        }
    }

    struct FakeVm {
        args: Vec<String>,
        floats: Vec<f64>,
        ints: Vec<i32>,
        self_reference: i32,
        origin: Vec3,
    }

    impl QcEnvVm for FakeVm {
        fn arg_string(&self, index: usize) -> Result<String, GuestError> {
            self.args
                .get(index)
                .cloned()
                .ok_or_else(|| GuestError::invalid("Missing builtin argument"))
        }

        fn var_string(&self, index: usize) -> String {
            self.args.get(index..).map(|args| args.join(" ")).unwrap_or_default()
        }

        fn return_float(&mut self, value: f64) {
            self.floats.push(value);
        }

        fn return_int(&mut self, value: i32) {
            self.ints.push(value);
        }

        fn global_int(&self, _name: &str) -> Result<i32, GuestError> {
            Ok(self.self_reference)
        }

        fn global_float(&self, _name: &str) -> Result<f64, GuestError> {
            Ok(4.0)
        }

        fn entity_vector(&self, _reference: i32, _field: &str) -> Result<Vec3, GuestError> {
            Ok(self.origin)
        }
    }

    fn declaration() -> ModCallbackDeclaration {
        ModCallbackDeclaration {
            program: Some(ModProgramRef {
                path: "progs.dat".to_string(),
                digest: "abc".to_string(),
            }),
            clients: Some(ModClientDeclaration {
                maximum: 8,
                ..ModClientDeclaration::default()
            }),
            cvars: vec![ModCvar {
                name: "skill".to_string(),
                value: "3".to_string(),
            }],
            ..ModCallbackDeclaration::default()
        }
    }

    fn environment() -> QcModEnvironment<FakeServices, FakeVisibility> {
        let program = FakeProgram {
            globals: HashMap::new(),
        };
        QcModEnvironment::new(
            FakeServices {
                printed: Vec::new(),
                clients: HashMap::new(),
                references: Vec::new(),
            },
            Some(FakeVisibility {
                next: None,
                checks: 0,
                state: 0,
            }),
            &program,
            &declaration(),
        )
        .unwrap()
    }

    #[test]
    fn defaults_and_declared_cvars() {
        let environment = environment();
        assert_eq!(environment.cvars().variable_value("skill"), 3.0);
        assert_eq!(environment.cvars().variable_value("maxclients"), 8.0);
        assert_eq!(environment.cvars().variable_value("sv_gravity"), 800.0);
        assert_eq!(environment.cvars().variable_string("sv_aim"), "0.93");
        assert!(environment.has_visibility());
    }

    #[test]
    fn host_builtins_read_write_and_print() {
        let mut environment = environment();
        environment.cvars_mut().set("developer", "1").unwrap();
        let mut vm = FakeVm {
            args: vec!["skill".to_string()],
            floats: Vec::new(),
            ints: Vec::new(),
            self_reference: 2,
            origin: vec3(0.0, 0.0, 0.0),
        };
        environment.call_host(QcEnvBuiltin::Cvar, &mut vm).unwrap();
        assert_eq!(vm.floats, vec![3.0]);
        let mut vm = FakeVm {
            args: vec!["skill".to_string(), "2".to_string()],
            floats: Vec::new(),
            ints: Vec::new(),
            self_reference: 2,
            origin: vec3(0.0, 0.0, 0.0),
        };
        environment.call_host(QcEnvBuiltin::CvarSet, &mut vm).unwrap();
        assert_eq!(environment.cvars().variable_value("skill"), 2.0);
        let mut vm = FakeVm {
            args: vec!["hello".to_string()],
            floats: Vec::new(),
            ints: Vec::new(),
            self_reference: 2,
            origin: vec3(0.0, 0.0, 0.0),
        };
        environment.call_host(QcEnvBuiltin::Dprint, &mut vm).unwrap();
        assert_eq!(environment.services().printed, vec!["hello".to_string()]);
        assert_eq!(QcEnvBuiltin::from_name("checkclient"), Some(QcEnvBuiltin::Checkclient));
        assert_eq!(QcEnvBuiltin::from_name("nope"), None);
    }

    #[test]
    fn checkclient_routes_through_visibility() {
        let owner = IdentityOwner::create("env").unwrap();
        let actor = owner.actor(5, 1);
        let mut with_target = QcModEnvironment::new(
            FakeServices {
                printed: Vec::new(),
                clients: HashMap::new(),
                references: Vec::new(),
            },
            Some(FakeVisibility {
                next: Some(actor),
                checks: 0,
                state: 0,
            }),
            &FakeProgram {
                globals: HashMap::new(),
            },
            &declaration(),
        )
        .unwrap();
        let mut vm = FakeVm {
            args: Vec::new(),
            floats: Vec::new(),
            ints: Vec::new(),
            self_reference: 2,
            origin: vec3(1.0, 2.0, 3.0),
        };
        with_target.call_host(QcEnvBuiltin::Checkclient, &mut vm).unwrap();
        assert_eq!(vm.ints, vec![5]);
        assert_eq!(with_target.services().references.len(), 1);
    }

    #[test]
    fn checkclient_requires_visibility() {
        let program = FakeProgram {
            globals: HashMap::new(),
        };
        let mut environment: QcModEnvironment<FakeServices, FakeVisibility> = QcModEnvironment::new(
            FakeServices {
                printed: Vec::new(),
                clients: HashMap::new(),
                references: Vec::new(),
            },
            None,
            &program,
            &declaration(),
        )
        .unwrap();
        let mut vm = FakeVm {
            args: Vec::new(),
            floats: Vec::new(),
            ints: Vec::new(),
            self_reference: 2,
            origin: vec3(0.0, 0.0, 0.0),
        };
        assert!(environment.call_host(QcEnvBuiltin::Checkclient, &mut vm).is_err());
    }

    #[test]
    fn initialize_globals_writes_declared_names() {
        let mut environment = environment();
        let mut machine = FakeMachine {
            program: FakeProgram {
                globals: [
                    ("skill".to_string(), QcValueType::Float),
                    ("time".to_string(), QcValueType::Float),
                    ("mapname".to_string(), QcValueType::String),
                ]
                .into_iter()
                .collect(),
            },
            floats: HashMap::new(),
            ints: HashMap::new(),
            strings: Vec::new(),
        };
        environment.initialize_globals(&mut machine).unwrap();
        assert_eq!(machine.floats["skill"], 3.0);
        assert_eq!(machine.floats["time"], 4.0);
        assert_eq!(machine.strings, vec!["e1m1".to_string()]);
    }

    #[test]
    fn client_lookup_and_checkpoint() {
        let owner = IdentityOwner::create("env-client").unwrap();
        let actor = owner.actor(2, 1);
        let mut environment = environment();
        environment.services_mut().clients.insert(
            actor.clone(),
            QcVisibilityClient {
                actor: actor.clone(),
                view_offset: vec3(0.0, 0.0, 8.0),
                notarget: true,
            },
        );
        let client = environment.client(&actor).unwrap().unwrap();
        assert!(client.notarget);
        assert!(environment.client(&owner.actor(9, 1)).unwrap().is_none());
        let saved = environment.capture();
        environment.cvars_mut().set("skill", "0").unwrap();
        environment.restore(&saved).unwrap();
        assert_eq!(environment.cvars().variable_value("skill"), 3.0);
    }
}
