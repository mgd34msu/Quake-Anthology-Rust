use qa_core::primitives::{
    Body, CallbackId, DamageEvent, DamageFlags, EntityId, PlayerState, Vec3,
};
use qa_gameplay::{
    combat::*,
    rules::{q1, q2, q3},
};

const TARGET: EntityId = EntityId {
    slot: 1,
    generation: 1,
};
const ATTACKER: EntityId = EntityId {
    slot: 2,
    generation: 1,
};

fn hit(amount: f32) -> DamageEvent {
    DamageEvent {
        target: TARGET,
        attacker: Some(ATTACKER),
        inflictor: Some(ATTACKER),
        amount,
        knockback: amount as i32,
        direction: Some(Vec3([1.0, 0.0, 0.0])),
        point: Vec3([10.0, 0.0, 0.0]),
        flags: DamageFlags::default(),
    }
}

struct Victim {
    player: PlayerState,
    traits: CombatTraits,
    cells: i32,
    calls: Vec<(CallbackId, Reaction, i32)>,
}

impl Victim {
    fn new() -> Self {
        Self {
            player: PlayerState {
                health: 100,
                armor: 100,
                armor_absorption: 0.3,
                armor_energy_absorption: 0.0,
                ..PlayerState::default()
            },
            traits: CombatTraits {
                client: true,
                motion: Motion::Walk,
                position: Vec3([10.0, 0.0, 0.0]),
                pain: Some(CallbackId(8)),
                die: Some(CallbackId(9)),
                ..CombatTraits::default()
            },
            cells: 100,
            calls: Vec::new(),
        }
    }

    fn apply(
        &mut self,
        rules: &DamageRules,
        event: DamageEvent,
        context: DamageContext,
    ) -> DamageResult {
        let mut target = DamageTarget {
            health: &mut self.player.health,
            armor: &mut self.player.armor,
            absorption: &mut self.player.armor_absorption,
            energy_absorption: self.player.armor_energy_absorption,
            power_cells: &mut self.cells,
            velocity: &mut self.player.body.velocity,
            traits: &mut self.traits,
        };
        apply_damage(
            rules,
            &mut target,
            event,
            context,
            |id, target, _, result| {
                self.calls.push((id, result.reaction, *target.health));
            },
        )
    }
}

#[test]
fn q1_ceil_quad_armor_exhaustion_and_zero_damage_pain_match_qc() {
    let mut v = Victim::new();
    v.player.armor = 3;
    let out = v.apply(
        &q1::DAMAGE,
        hit(3.25),
        DamageContext {
            attacker_multiplier: 4.0,
            ..DamageContext::default()
        },
    );
    assert_eq!(
        (out.armor_saved, out.health_damage, v.player.health),
        (3, 10, 90)
    );
    assert_eq!((v.player.armor, v.player.armor_absorption), (0, 0.0));
    assert_eq!(v.player.body.velocity, Vec3([104.0, 0.0, 0.0]));
    assert_eq!(v.calls, [(CallbackId(8), Reaction::Pain, 90)]);
    let mut v = Victim::new();
    v.player.armor_absorption = 0.8;
    let out = v.apply(&q1::DAMAGE, hit(1.0), DamageContext::default());
    assert_eq!((out.armor_saved, out.health_damage), (1, 0));
    assert_eq!(v.calls, [(CallbackId(8), Reaction::Pain, 100)]);
}

#[test]
fn q1_protection_is_after_armor_and_momentum() {
    for god in [false, true] {
        let mut v = Victim::new();
        v.traits.god = god;
        v.traits.invincible = !god;
        let out = v.apply(&q1::DAMAGE, hit(20.0), DamageContext::default());
        assert!(out.blocked);
        assert_eq!((v.player.health, v.player.armor), (100, 94));
        assert_eq!(v.player.body.velocity, Vec3([160.0, 0.0, 0.0]));
        assert!(v.calls.is_empty());
    }
}

#[test]
fn q2_normal_energy_power_armor_and_original_cell_rounding() {
    let mut v = Victim::new();
    v.player.armor_absorption = 0.8;
    v.player.armor_energy_absorption = 0.6;
    let normal = v.apply(&q2::DAMAGE, hit(21.0), DamageContext::default());
    assert_eq!((normal.armor_saved, normal.health_damage), (17, 4));
    let mut event = hit(21.0);
    event.flags = DamageFlags(DamageFlags::ENERGY);
    let energy = v.apply(&q2::DAMAGE, event, DamageContext::default());
    assert_eq!((energy.armor_saved, energy.health_damage), (13, 8));
    v.traits.power_armor = PowerArmor::Shield;
    let shield = v.apply(&q2::DAMAGE, hit(31.0), DamageContext::default());
    assert_eq!(
        (
            shield.power_saved,
            shield.armor_saved,
            shield.health_damage,
            v.cells
        ),
        (20, 9, 2, 90)
    );
    let one = v.apply(&q2::DAMAGE, hit(2.0), DamageContext::default());
    assert_eq!(
        (one.power_saved, one.armor_saved, one.health_damage, v.cells),
        (1, 1, 0, 90)
    );
    v.traits.power_armor = PowerArmor::Screen;
    event.point = Vec3([20.0, 0.0, 0.0]);
    let front = v.apply(&q2::DAMAGE, event, DamageContext::default());
    assert_eq!(
        (
            front.power_saved,
            front.armor_saved,
            front.health_damage,
            v.cells
        ),
        (7, 9, 5, 83)
    );
    event.point = Vec3([0.0, 0.0, 0.0]);
    assert_eq!(
        v.apply(&q2::DAMAGE, event, DamageContext::default())
            .power_saved,
        0
    );
}

#[test]
fn q2_protection_precedes_armor_and_self_knockback_keeps_rocket_jump_scale() {
    let mut v = Victim::new();
    v.traits.god = true;
    let out = v.apply(
        &q2::DAMAGE,
        hit(20.0),
        DamageContext {
            self_hit: true,
            ..DamageContext::default()
        },
    );
    assert_eq!(
        (out.armor_saved, out.health_damage, v.player.armor, v.cells),
        (20, 0, 100, 100)
    );
    assert_eq!(v.player.body.velocity, Vec3([160.0, 0.0, 0.0]));
    assert!(v.calls.is_empty());
    v.traits.god = false;
    let out = v.apply(
        &q2::DAMAGE,
        hit(9.0),
        DamageContext {
            easy_single_player: true,
            ..DamageContext::default()
        },
    );
    assert_eq!((out.armor_saved, out.health_damage), (2, 2));
    v.traits.client = false;
    v.traits.monster = true;
    let out = v.apply(
        &q2::DAMAGE,
        hit(9.0),
        DamageContext {
            attacker_client: true,
            ..DamageContext::default()
        },
    );
    assert_eq!((out.armor_saved, out.health_damage), (0, 18));
}

#[test]
fn q3_double_literal_armor_self_damage_and_knockback_order() {
    let mut v = Victim::new();
    let out = v.apply(
        &q3::DAMAGE,
        hit(100.0),
        DamageContext {
            self_hit: true,
            ..DamageContext::default()
        },
    );
    assert_eq!(
        (out.knockback, out.armor_saved, out.health_damage),
        (100, 33, 17)
    );
    assert_eq!(v.player.body.velocity, Vec3([500.0, 0.0, 0.0]));
    assert_eq!(v.traits.knockback_time_ms, 200);
    let mut v = Victim::new();
    let out = v.apply(
        &q3::DAMAGE,
        hit(25.0),
        DamageContext {
            attacker_client: true,
            attacker_handicap: 50,
            ..DamageContext::default()
        },
    );
    assert_eq!(
        (out.knockback, out.armor_saved, out.health_damage),
        (12, 8, 4)
    );
    assert_eq!(v.traits.knockback_time_ms, 50);
    let mut v = Victim::new();
    let out = v.apply(&q3::DAMAGE, hit(100.0), DamageContext::default());
    assert_eq!((out.armor_saved, out.health_damage), (66, 34));
}

#[test]
fn q3_team_protection_keeps_knockback_battlesuit_radius_blocks_after_knockback() {
    let mut v = Victim::new();
    let out = v.apply(
        &q3::DAMAGE,
        hit(20.0),
        DamageContext {
            same_team: true,
            prevent_team_damage: true,
            ..DamageContext::default()
        },
    );
    assert!(out.blocked);
    assert_eq!((v.player.health, v.player.armor), (100, 100));
    assert_eq!(v.player.body.velocity, Vec3([100.0, 0.0, 0.0]));
    assert!(v.calls.is_empty());
    v.traits.battlesuit = true;
    let mut event = hit(20.0);
    event.flags = DamageFlags(DamageFlags::RADIUS);
    assert!(
        v.apply(&q3::DAMAGE, event, DamageContext::default())
            .blocked
    );
    assert_eq!(v.player.body.velocity, Vec3([200.0, 0.0, 0.0]));
    assert_eq!(v.player.armor, 100);
}

#[test]
fn death_uses_entity_callback_and_preserves_native_health_limits() {
    for (rules, minimum) in [(&q1::DAMAGE, -99), (&q2::DAMAGE, -999), (&q3::DAMAGE, -999)] {
        let mut v = Victim::new();
        v.player.armor = 0;
        let out = v.apply(rules, hit(2000.0), DamageContext::default());
        assert_eq!(out.reaction, Reaction::Death);
        assert_eq!(v.player.health, minimum);
        assert_eq!(v.calls, [(CallbackId(9), Reaction::Death, minimum)]);
    }
}

#[test]
fn attack_radius_rules_and_target_armor_rules_are_independent() {
    let blast = Blast {
        origin: Vec3::default(),
        amount: 100.0,
        radius: 100.0,
        attacker: Some(ATTACKER),
        inflictor: Some(ATTACKER),
        ignore: None,
    };
    let target = BlastTarget {
        id: TARGET,
        body: Body {
            position: Vec3([50.0, 0.0, 0.0]),
            mins: Vec3([-10.0; 3]),
            maxs: Vec3([10.0; 3]),
            ..Body::default()
        },
        damageable: true,
        multiplier: 1.0,
    };
    let mut v = Victim::new();
    let mut emitted = 0;
    radius_damage(
        &q1::DAMAGE,
        blast,
        [target],
        |id, origin| {
            assert_eq!(id, TARGET);
            assert_eq!(origin, blast.origin);
            true
        },
        |event| {
            emitted += 1;
            assert_eq!(event.amount, 75.0);
            let out = v.apply(&q3::DAMAGE, event, DamageContext::default());
            assert_eq!((out.armor_saved, out.health_damage), (50, 25));
        },
    );
    assert_eq!(emitted, 1);
    for (rules, expected) in [
        (&q1::DAMAGE, 75.0),
        (&q2::DAMAGE, 75.0),
        (&q3::DAMAGE, 60.0),
    ] {
        radius_damage(
            rules,
            blast,
            [target],
            |_, _| true,
            |event| assert_eq!(event.amount, expected),
        );
        radius_damage(
            rules,
            blast,
            [target],
            |_, _| false,
            |_| panic!("occluded target emitted"),
        );
        radius_damage(
            rules,
            Blast {
                ignore: Some(TARGET),
                ..blast
            },
            [target],
            |_, _| true,
            |_| panic!("ignored target emitted"),
        );
    }
    let mut self_damage = 0.0;
    radius_damage(
        &q2::DAMAGE,
        Blast {
            attacker: Some(TARGET),
            ..blast
        },
        [target],
        |_, _| true,
        |event| self_damage = event.amount,
    );
    assert_eq!(self_damage, 37.0);
}
