//! Native Quake I plats-family movers: `misc_teleporttrain`.
//!
//! Stock `use`/`think`/mover gamecode for the end-map spike train —
//! `func_train_find`, `train_next`, `train_wait`, `train_use` — driven
//! from [`Q1NativeBehaviors`] through the native think/mover hooks.
//! `func_train` (brush riders on other maps) is a later slice; this
//! module owns the shared record shape it will extend.
//!
//! qsrc: `progs106/plats.qc` (blocked 221, use 228, wait 235, next 248,
//! find 264, `func_train` 290, `misc_teleporttrain` 335),
//! `WinQuake/model.c:1645` (alias models size to +/-16).

use qa_core::math::{vec3, Bounds, Vec3};
use qa_world::movers::{MoverKind, MoverPhase, MoverState, MoverTable};
use qa_world::server::{Server, ServerLogic};
use qa_world::session::Simulation;
use qa_world::spawn::{SpawnFields, SpawnRegistry};
use qa_world::WorldError;

use super::native_q1_spawns::{q1_rearm_travel, q1_redirect_mover, Q1NativeBehaviors};
use super::native_q1_triggers::Q1ThinkKind;

/// Spiked-sphere model the teleport train rides (`plats.qc:355`).
pub const Q1_TELEPORTTRAIN_MODEL: &str = "progs/teleport.mdl";
/// Engine alias-model bounds (`model.c:1645`): every alias model sizes
/// to +/-16, so the train offsets corners by 16 up each axis.
pub const Q1_TELEPORTTRAIN_MIN: Vec3 = Vec3 {
    x: -16.0,
    y: -16.0,
    z: -16.0,
};
/// Engine alias-model bounds (`model.c:1645`).
pub const Q1_TELEPORTTRAIN_MAX: Vec3 = Vec3 {
    x: 16.0,
    y: 16.0,
    z: 16.0,
};
/// Stock spin (`plats.qc:347`): the spike turns 100/200/300 deg/s.
/// The native mover engine has no angular channel and the scheduler
/// holds one think per actor (stock's single `think` slot), so the
/// sim carries the rate for the presentation slice and the body holds
/// its spawn angles instead of stepping.
pub const Q1_TELEPORTTRAIN_AVELOCITY: Vec3 = Vec3 {
    x: 100.0,
    y: 200.0,
    z: 300.0,
};
/// Default travel speed (`plats.qc:338`).
pub const Q1_TRAIN_DEFAULT_SPEED: f64 = 100.0;

/// Live `misc_teleporttrain` record (`plats.qc:335`): the end-map spike
/// that loops its `path_corner` route. The kill teleporter targets the
/// train itself, so arrivals track its live origin.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Train {
    /// Next corner's targetname (`self.target`, advanced every leg).
    pub target: Option<String>,
    /// Travel speed in units per second.
    pub speed: f64,
    /// Whether a targetname holds the start until fired.
    pub targeted: bool,
    /// Whether `train_next` ran (`think != func_train_find`): later
    /// uses return early (`plats.qc:228`).
    pub activated: bool,
    /// Visual spin rate; the body holds still (see the constant).
    pub avelocity: Vec3,
}

/// Register the native `misc_teleporttrain` spawn function. Trains
/// spawn bodied at the map origin for [`build_q1_train`] to size.
pub fn register_q1_plat_spawns(registry: &mut SpawnRegistry) {
    registry.register(
        "misc_teleporttrain",
        Box::new(|fields| {
            Ok(qa_world::spawn::SpawnRequest {
                definition: "q1:misc_teleporttrain".to_string(),
                origin: Some(fields.origin),
                combat: None,
                grants: Vec::new(),
            })
        }),
    );
}

/// Finish a spawned `misc_teleporttrain` actor: size its body to the
/// engine alias bounds, register its pusher mover, record its route
/// state, and arm `func_train_find` 0.1 s out. A missing target fails
/// the build (stock `objerror`s `func_train without a target`).
pub fn build_q1_train<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    actor: &qa_core::identity::OwnedActor,
    fields: &SpawnFields,
) -> Result<(), WorldError> {
    let Some(target) = fields.target.clone() else {
        return Err(WorldError::BadSpawnFields(
            "misc_teleporttrain without a target".to_string(),
        ));
    };
    let mut speed = fields
        .extra
        .get("speed")
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or(Q1_TRAIN_DEFAULT_SPEED);
    if speed == 0.0 {
        speed = Q1_TRAIN_DEFAULT_SPEED;
    }
    server.simulation_mut().set_body_bounds(
        actor.id(),
        Bounds {
            min: Q1_TELEPORTTRAIN_MIN,
            max: Q1_TELEPORTTRAIN_MAX,
        },
    )?;
    let origin = server
        .simulation()
        .body_state(actor.id())
        .map_or(fields.origin, |body| body.origin);
    // `SOLID_NOT` (`plats.qc:343`): the spike ball ghosts through
    // Shub and the world, carrying nothing and blocked by nothing.
    let mut mover = MoverState::new(MoverKind::Pusher, origin, origin, speed, 0.0);
    mover.solid = false;
    server.movers_mut().insert(actor.id().clone(), mover);
    behaviors.trains.insert(
        actor.id(),
        Q1Train {
            target: Some(target),
            speed,
            targeted: fields.targetname.as_deref().is_some_and(|name| !name.is_empty()),
            activated: false,
            avelocity: Q1_TELEPORTTRAIN_AVELOCITY,
        },
    );
    let now = server.simulation().frame().time.as_seconds_f64();
    behaviors.schedule_think(actor.id(), Q1ThinkKind::TrainFind, now + 0.1);
    Ok(())
}

/// First targetname match in spawn order (stock `find`).
fn q1_train_corner(behaviors: &Q1NativeBehaviors, target: &str) -> Option<qa_core::identity::ActorId> {
    behaviors.by_targetname.get(target)?.first().cloned()
}

/// Run `func_train_find` (`plats.qc:264`): adopt the first corner's
/// onward link, plant the origin on the corner minus the model mins,
/// and roll at once unless a targetname holds the start. A missing
/// corner stops the train (stock `objerror`s; the loader degrades).
pub fn q1_train_find(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    actor: &qa_core::identity::ActorId,
) {
    let target = behaviors.trains.get(actor).and_then(|train| train.target.clone());
    let Some(target) = target else {
        return;
    };
    let Some(corner) = q1_train_corner(behaviors, &target) else {
        return;
    };
    let onward = behaviors
        .movetargets
        .get(&corner)
        .and_then(|movetarget| movetarget.target.clone());
    let corner_origin = simulation.body_state(&corner).map_or_else(
        || {
            simulation
                .body_state(actor)
                .map_or(vec3(0.0, 0.0, 0.0), |body| body.origin)
        },
        |body| body.origin,
    );
    if let Some(train) = behaviors.trains.get_mut(actor) {
        train.target = onward;
    }
    let _ignored = simulation.set_body_origin(
        actor,
        vec3(
            corner_origin.x - Q1_TELEPORTTRAIN_MIN.x,
            corner_origin.y - Q1_TELEPORTTRAIN_MIN.y,
            corner_origin.z - Q1_TELEPORTTRAIN_MIN.z,
        ),
    );
    let targeted = behaviors.trains.get(actor).is_some_and(|train| train.targeted);
    if !targeted {
        q1_train_next(behaviors, simulation, movers, actor);
    }
}

/// Run `train_next` (`plats.qc:248`): advance the route link past the
/// next corner and roll toward that corner minus the model mins at
/// train speed (`SUB_CalcMove`). A missing corner, or a corner with
/// no onward link, stops the train (stock `objerror`s). The null
/// noises stay silent: both train samples are `misc/null.wav`.
pub fn q1_train_next(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    actor: &qa_core::identity::ActorId,
) {
    let target = behaviors.trains.get(actor).and_then(|train| train.target.clone());
    let Some(target) = target else {
        return;
    };
    let Some(corner) = q1_train_corner(behaviors, &target) else {
        return;
    };
    let onward = behaviors
        .movetargets
        .get(&corner)
        .and_then(|movetarget| movetarget.target.clone());
    let corner_origin = simulation.body_state(&corner).map_or_else(
        || {
            simulation
                .body_state(actor)
                .map_or(vec3(0.0, 0.0, 0.0), |body| body.origin)
        },
        |body| body.origin,
    );
    let origin = simulation
        .body_state(actor)
        .map_or(vec3(0.0, 0.0, 0.0), |body| body.origin);
    if let Some(train) = behaviors.trains.get_mut(actor) {
        train.target.clone_from(&onward);
        train.activated = true;
    }
    if onward.is_none() {
        return;
    }
    let destination = vec3(
        corner_origin.x - Q1_TELEPORTTRAIN_MIN.x,
        corner_origin.y - Q1_TELEPORTTRAIN_MIN.y,
        corner_origin.z - Q1_TELEPORTTRAIN_MIN.z,
    );
    if let Some(state) = movers.get_mut(actor) {
        state.pos1 = origin;
        state.pos2 = destination;
        q1_redirect_mover(state, origin, true);
    }
}

/// Run `train_wait` (`plats.qc:235`) inline at each leg arrival: the
/// next leg rolls 0.1 s out. Stock waits `targ.wait` with the null
/// noise, but no retail train route sets a corner wait (verified
/// across the pak `path_corner` lumps), so the wait branch stays out
/// until `func_train` lands with its own routes.
pub fn q1_train_wait(behaviors: &mut Q1NativeBehaviors, simulation: &Simulation, actor: &qa_core::identity::ActorId) {
    let now = simulation.frame().time.as_seconds_f64();
    behaviors.schedule_think(actor, Q1ThinkKind::TrainNext, now + 0.1);
}

/// Run `train_use` (`plats.qc:228`): an unactivated train rolls at
/// once; a running train ignores later fires.
pub fn q1_train_use(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    actor: &qa_core::identity::ActorId,
) {
    let activated = behaviors.trains.get(actor).is_some_and(|train| train.activated);
    if activated {
        return;
    }
    q1_train_next(behaviors, simulation, movers, actor);
}

/// Route mover thinks into `train_wait`: arrival runs the corner
/// pause, while a mid-travel think without arrival re-arms the
/// arrival think from the live origin like doors — float dust
/// between the armed instant and the corner would otherwise consume
/// the think and strand the train with no future think (the engine
/// holds still without one).
pub fn q1_train_mover_think(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut MoverTable,
    actor: &qa_core::identity::ActorId,
    phase: MoverPhase,
    arrived: bool,
) {
    if !behaviors.trains.contains_key(actor) {
        return;
    }
    match (phase, arrived) {
        (MoverPhase::AtPos2, true) => q1_train_wait(behaviors, simulation, actor),
        (MoverPhase::ToPos1 | MoverPhase::ToPos2, _) => q1_rearm_travel(simulation, movers, actor),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ActorId;

    use super::*;
    use crate::options::ApplicationOptions;
    use crate::startup::{open_server, StartupConfig};

    fn test_server() -> Server<qa_guest::server::GuestServerLogic> {
        let config = StartupConfig::from_options(&ApplicationOptions::default()).unwrap();
        open_server(&config).unwrap()
    }

    fn register_all(server: &mut Server<qa_guest::server::GuestServerLogic>) {
        super::super::native_q1_spawns::register_q1_spawns(server.spawns_mut());
        super::super::native_q1_monsters::register_q1_monster_spawns(server.spawns_mut());
        register_q1_plat_spawns(server.spawns_mut());
    }

    fn train_fields(pairs: &[(&str, &str)]) -> SpawnFields {
        let mut full = vec![("classname", "misc_teleporttrain")];
        full.extend_from_slice(pairs);
        SpawnFields::parse(&full).unwrap()
    }

    fn spawn_corner(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        targetname: &str,
        origin: &str,
        target: Option<&str>,
    ) -> ActorId {
        let mut pairs = vec![
            ("classname", "path_corner"),
            ("targetname", targetname),
            ("origin", origin),
        ];
        if let Some(next) = target {
            pairs.push(("target", next));
        }
        let fields = SpawnFields::parse(&pairs).unwrap();
        let actor = server.spawn_entity(&fields).unwrap();
        super::super::native_q1_monsters::build_q1_movetarget(server, behaviors, &actor, &fields).unwrap();
        super::super::native_q1_triggers::q1_note_targetname(behaviors, &fields, actor.id());
        actor.id().clone()
    }

    fn spawn_train(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> ActorId {
        let actor = server.spawn_entity(fields).unwrap();
        build_q1_train(server, behaviors, &actor, fields).unwrap();
        super::super::native_q1_triggers::q1_note_targetname(behaviors, fields, actor.id());
        actor.id().clone()
    }

    fn run_find(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        train: &ActorId,
    ) {
        let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
        q1_train_find(behaviors, simulation, movers, train);
    }

    #[test]
    fn build_sizes_registers_mover_and_arms_find() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let fields = train_fields(&[
            ("origin", "10 20 30"),
            ("target", "c1"),
            ("targetname", "t0"),
            ("speed", "200"),
        ]);
        let train = spawn_train(&mut server, &mut behaviors, &fields);
        let record = behaviors.trains.get(&train).unwrap();
        assert_eq!(record.target, Some("c1".to_string()));
        assert_eq!(record.speed, 200.0);
        assert!(record.targeted);
        assert!(!record.activated);
        assert_eq!(record.avelocity, Q1_TELEPORTTRAIN_AVELOCITY);
        let body = server.simulation().body_state(&train).unwrap();
        assert_eq!(body.origin, vec3(10.0, 20.0, 30.0));
        assert_eq!(body.bounds.min, Q1_TELEPORTTRAIN_MIN);
        assert_eq!(body.bounds.max, Q1_TELEPORTTRAIN_MAX);
        assert!(!behaviors.solids.contains(&train), "SOLID_NOT stays out of the scene");
        let mover = server.movers_mut().get(&train).unwrap();
        assert_eq!(mover.kind, MoverKind::Pusher);
        assert!(!mover.solid, "SOLID_NOT ghosts through the world");
        assert_eq!(mover.phase, MoverPhase::AtPos1);
        assert_eq!(mover.pos1, vec3(10.0, 20.0, 30.0));
        assert_eq!(mover.pos2, vec3(10.0, 20.0, 30.0));
        assert_eq!(behaviors.thinks.len(), 1);
        assert!(matches!(behaviors.thinks[0].kind, Q1ThinkKind::TrainFind));
        assert!((behaviors.thinks[0].due_seconds - 0.1).abs() < 1e-9);
    }

    #[test]
    fn build_defaults_speed_and_requires_target() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        let plain = train_fields(&[("target", "c1")]);
        let actor = server.spawn_entity(&plain).unwrap();
        build_q1_train(&mut server, &mut behaviors, &actor, &plain).unwrap();
        assert_eq!(behaviors.trains.get(actor.id()).unwrap().speed, 100.0);
        assert!(!behaviors.trains.get(actor.id()).unwrap().targeted);
        let zero = train_fields(&[("target", "c1"), ("speed", "0")]);
        let actor = server.spawn_entity(&zero).unwrap();
        build_q1_train(&mut server, &mut behaviors, &actor, &zero).unwrap();
        assert_eq!(behaviors.trains.get(actor.id()).unwrap().speed, 100.0);
        let missing = train_fields(&[]);
        let actor = server.spawn_entity(&missing).unwrap();
        assert!(build_q1_train(&mut server, &mut behaviors, &actor, &missing).is_err());
        assert!(!behaviors.trains.contains_key(actor.id()));
    }

    #[test]
    fn find_plants_on_first_corner_and_waits_when_targeted() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_corner(&mut server, &mut behaviors, "c1", "100 0 0", Some("c2"));
        spawn_corner(&mut server, &mut behaviors, "c2", "300 0 0", Some("c1"));
        let train = spawn_train(
            &mut server,
            &mut behaviors,
            &train_fields(&[("target", "c1"), ("targetname", "t0"), ("speed", "200")]),
        );
        run_find(&mut server, &mut behaviors, &train);
        let record = behaviors.trains.get(&train).unwrap();
        assert_eq!(record.target, Some("c2".to_string()));
        assert!(!record.activated, "targeted trains wait for use");
        let body = server.simulation().body_state(&train).unwrap();
        assert_eq!(body.origin, vec3(116.0, 16.0, 16.0), "corner minus model mins");
        let mover = server.movers_mut().get(&train).unwrap();
        assert_eq!(mover.phase, MoverPhase::AtPos1, "no travel before use");
    }

    #[test]
    fn find_rolls_at_once_without_targetname() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_corner(&mut server, &mut behaviors, "c1", "100 0 0", Some("c2"));
        spawn_corner(&mut server, &mut behaviors, "c2", "300 0 0", Some("c1"));
        let train = spawn_train(
            &mut server,
            &mut behaviors,
            &train_fields(&[("target", "c1"), ("speed", "200")]),
        );
        run_find(&mut server, &mut behaviors, &train);
        let record = behaviors.trains.get(&train).unwrap();
        assert_eq!(record.target, Some("c1".to_string()), "find plus next advance twice");
        assert!(record.activated);
        let mover = server.movers_mut().get(&train).unwrap();
        assert_eq!(mover.phase, MoverPhase::ToPos2);
        assert_eq!(mover.pos1, vec3(116.0, 16.0, 16.0));
        assert_eq!(mover.pos2, vec3(316.0, 16.0, 16.0), "next corner minus model mins");
    }

    #[test]
    fn use_starts_once_and_later_fires_return_early() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_corner(&mut server, &mut behaviors, "c1", "100 0 0", Some("c2"));
        spawn_corner(&mut server, &mut behaviors, "c2", "300 0 0", Some("c1"));
        let train = spawn_train(
            &mut server,
            &mut behaviors,
            &train_fields(&[("target", "c1"), ("targetname", "t0"), ("speed", "200")]),
        );
        run_find(&mut server, &mut behaviors, &train);
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_train_use(&mut behaviors, simulation, movers, &train);
        }
        assert_eq!(behaviors.trains.get(&train).unwrap().target, Some("c1".to_string()));
        let before = server.movers_mut().get(&train).unwrap().clone();
        assert_eq!(before.phase, MoverPhase::ToPos2);
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_train_use(&mut behaviors, simulation, movers, &train);
        }
        assert_eq!(
            behaviors.trains.get(&train).unwrap().target,
            Some("c1".to_string()),
            "second use advances nothing"
        );
        let after = server.movers_mut().get(&train).unwrap().clone();
        assert_eq!(after.pos2, before.pos2);
        assert_eq!(after.next_think_seconds, before.next_think_seconds);
    }

    #[test]
    fn arrival_schedules_next_and_missing_corners_stop() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_corner(&mut server, &mut behaviors, "c1", "100 0 0", Some("c2"));
        spawn_corner(&mut server, &mut behaviors, "c2", "300 0 0", Some("c1"));
        let train = spawn_train(
            &mut server,
            &mut behaviors,
            &train_fields(&[("target", "c1"), ("speed", "200")]),
        );
        run_find(&mut server, &mut behaviors, &train);
        behaviors.thinks.clear();
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_train_mover_think(
                &mut behaviors,
                simulation,
                movers,
                &train,
                MoverPhase::AtPos2,
                true,
            );
        }
        assert_eq!(behaviors.thinks.len(), 1);
        assert!(matches!(behaviors.thinks[0].kind, Q1ThinkKind::TrainNext));
        assert!((behaviors.thinks[0].due_seconds - 0.1).abs() < 1e-9);
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_train_mover_think(
                &mut behaviors,
                simulation,
                movers,
                &train,
                MoverPhase::AtPos2,
                false,
            );
        }
        assert_eq!(behaviors.thinks.len(), 1, "wait pings schedule nothing");
        // A mid-travel think without arrival re-arms the arrival think
        // instead of waiting: the consumed think strands the mover.
        server.movers_mut().get_mut(&train).unwrap().next_think_seconds = 0.0;
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_train_mover_think(
                &mut behaviors,
                simulation,
                movers,
                &train,
                MoverPhase::ToPos2,
                false,
            );
        }
        let rearmed = server.movers_mut().get(&train).unwrap();
        assert!(
            rearmed.next_think_seconds > rearmed.local_time_seconds,
            "mid-travel thinks re-arm the arrival"
        );
        assert_eq!(behaviors.thinks.len(), 1, "re-arming schedules no leg");
        let lost = spawn_train(
            &mut server,
            &mut behaviors,
            &train_fields(&[("target", "void"), ("targetname", "t1")]),
        );
        behaviors.thinks.clear();
        run_find(&mut server, &mut behaviors, &lost);
        assert_eq!(behaviors.trains.get(&lost).unwrap().target, Some("void".to_string()));
        assert!(behaviors.thinks.is_empty(), "missing corner stops the train");
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_train_next(&mut behaviors, simulation, movers, &lost);
        }
        assert!(behaviors.thinks.is_empty());
        assert!(!behaviors.trains.get(&lost).unwrap().activated);
    }

    #[test]
    fn dead_end_corner_advances_to_none_and_stops() {
        let mut server = test_server();
        register_all(&mut server);
        let mut behaviors = Q1NativeBehaviors::new();
        spawn_corner(&mut server, &mut behaviors, "c1", "100 0 0", None);
        let train = spawn_train(
            &mut server,
            &mut behaviors,
            &train_fields(&[("target", "c1"), ("targetname", "t0"), ("speed", "200")]),
        );
        run_find(&mut server, &mut behaviors, &train);
        assert_eq!(behaviors.trains.get(&train).unwrap().target, None);
        let planted = server.simulation().body_state(&train).unwrap().origin;
        assert_eq!(planted, vec3(116.0, 16.0, 16.0));
        {
            let (simulation, movers, _) = server.simulation_movers_and_triggers_mut();
            q1_train_use(&mut behaviors, simulation, movers, &train);
        }
        let mover = server.movers_mut().get(&train).unwrap();
        assert_eq!(mover.phase, MoverPhase::AtPos1, "dead end never rolls");
    }
}
