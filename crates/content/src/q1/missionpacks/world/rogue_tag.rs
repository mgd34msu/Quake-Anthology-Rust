//! Rogue tag-token lifecycle and scoring
//! (`src/content/q1/missionpacks/world/rogue-tag.ts`).
//!
//! dmatch.qc token lifecycle and source scoring.

use qa_core::identity::{ActorId, same_actor};
use qa_core::math::Vec3;

use crate::q1::base::provider::spawn_select;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{
    Q1Event, Q1MessageArg, Q1MoveType, Q1Powerup, Q1Solid, Q1TraceRequest, ZERO, vadd,
};
use crate::q1::{Q1Error, q1_error};

use super::common::{later, number};
use super::with_missionpack_hooks;

/// Announce a token event to every player (`announce`).
fn announce(game: &mut Q1EntityServices, text: &str, actor: &ActorId) -> Result<(), Q1Error> {
    let name = with_missionpack_hooks(game, |_, hooks| {
        let player_name = hooks
            .player_name
            .as_ref()
            .ok_or_else(|| q1_error("Rogue token messages require shared player names"))?;
        Ok(player_name(actor))
    })?;
    for player in (game.host.players)() {
        game.host.emit(Q1Event::Message {
            player,
            text: text.to_string(),
            center: false,
            args: Some(vec![Q1MessageArg::Text(name.clone())]),
            parts: None,
        });
    }
    Ok(())
}

/// Find the tag token (`token`).
fn token(game: &Q1EntityServices) -> Option<ActorId> {
    game.entity_ids().into_iter().find(|id| {
        game.entity(id)
            .is_some_and(|entity| entity.classname == "dmatch_tag_token")
    })
}

/// Drop the token to the floor (`dropFloor`).
fn drop_floor(game: &mut Q1EntityServices, id: &ActorId) -> Result<bool, Q1Error> {
    let body = game.body(id)?;
    let hit = game.host.trace(&Q1TraceRequest {
        start: body.origin,
        end: vadd(
            body.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: -256.0,
            },
        ),
        bounds: body.bounds,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    if hit.fraction == 1.0 || hit.all_solid {
        return Ok(false);
    }
    game.set_body(
        id,
        &BodyPatch {
            origin: Some(hit.end),
            ground: Some(hit.actor.clone()),
            ..Default::default()
        },
    )?;
    game.link(id)?;
    Ok(true)
}

/// Give the token to a player (`take`).
fn take(
    game: &mut Q1EntityServices,
    token: &ActorId,
    actor: &ActorId,
    announcement_delay: f64,
) -> Result<(), Q1Error> {
    if let Some(world) = game.world.clone() {
        let actor = actor.clone();
        game.update_entity(&world, |world| {
            world
                .references
                .insert("rogue:tag_token_owner".to_string(), Some(actor));
        })?;
    }
    let actor = actor.clone();
    let time = game.time;
    game.update_entity(token, |token| {
        token.owner = Some(actor);
        number(token, "tag_frags", 0.0);
        number(token, "tag_message_time", time + announcement_delay);
        token.solid = Q1Solid::None;
        token.touch = None;
    })?;
    later(game, token, 0.1, "rogue:tag_think")
}

/// Respawn the token at a spawn point (`respawn`).
fn respawn(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let token = token(game);
    let Some(token) = token else {
        return Ok(());
    };
    let point =
        spawn_select(game, true)?.ok_or_else(|| q1_error("Tag token has no respawn point"))?;
    let origin = game.body(&point)?.origin;
    game.set_origin(&token, origin)?;
    if let Some(world) = game.world.clone() {
        game.update_entity(&world, |world| {
            world
                .references
                .insert("rogue:tag_token_owner".to_string(), None);
        })?;
    }
    let touch_name = game.named.touch("rogue:tag_touch")?;
    let null = game.named.action("SUB_Null")?;
    game.update_entity(&token, |token| {
        token.solid = Q1Solid::Trigger;
        token.touch = Some(touch_name);
        token.think = Some(null);
        token.owner = None;
        number(token, "tag_frags", 0.0);
    })?;
    drop_floor(game, &token)?;
    Ok(())
}

/// Respawn the token from a scheduled dispatch.
fn tag_respawn(game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
    respawn(game)
}

/// Settle a dropped token, then time it out.
fn tag_fall(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| number(entity, "tag_frags", 0.0))?;
    drop_floor(game, id)?;
    later(game, id, 30.0, "rogue:tag_respawn")
}

/// Place a spawned token.
fn tag_place(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.movement = Q1MoveType::Toss;
        entity.solid = Q1Solid::Trigger;
    })?;
    let origin = game.body(id)?.origin;
    game.set_origin(
        id,
        vadd(
            origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 6.0,
            },
        ),
    )?;
    if drop_floor(game, id)? {
        return Ok(());
    }
    game.remove(id)
}

/// Track the token carrier.
fn tag_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let owner = game.entity(id).and_then(|entity| entity.owner.clone());
    if owner.as_ref().is_some_and(|owner| game.health(owner) > 0.0) {
        let owner = owner.expect("owner");
        if game
            .entity(id)
            .map(|entity| entity.number("tag_message_time"))
            .unwrap_or(0.0)
            < game.time
        {
            announce(game, "$qc_has_token", &owner)?;
            let time = game.time;
            game.update_entity(id, |entity| number(entity, "tag_message_time", time + 30.0))?;
        }
        let origin = game
            .host
            .bodies
            .read(&owner)
            .map(|body| body.origin)
            .unwrap_or(ZERO);
        game.set_origin(
            id,
            vadd(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 48.0,
                },
            ),
        )?;
        return later(game, id, 0.1, "rogue:tag_think");
    }
    if let Some(owner) = owner {
        announce(game, "$qc_lost_token", &owner)?;
    }
    game.update_entity(id, |entity| number(entity, "tag_frags", 0.0))?;
    let touch_name = game.named.touch("rogue:tag_touch")?;
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::Trigger;
        entity.owner = None;
        entity.touch = Some(touch_name);
    })?;
    later(game, id, 0.1, "rogue:tag_fall")
}

/// Claim the token on touch.
fn tag_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    let other = other.clone();
    take(game, id, &other, 30.0)?;
    game.sound_simple(id, "runes/end1.wav")?;
    announce(game, "$qc_got_token", &other)
}

/// Spawn a `dmatch_tag_token`.
fn spawn_token(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().teamplay.unwrap_or(0) != 3 {
        return game.remove(id);
    }
    game.update_entity(id, |entity| {
        entity.model = "progs/sphere.mdl".to_string();
        entity.skin = 1;
        entity.effects |= 8;
    })?;
    let touch_name = game.named.touch("rogue:tag_touch")?;
    game.update_entity(id, |entity| entity.touch = Some(touch_name))?;
    game.set_bounds(
        id,
        qa_core::math::Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -16.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 16.0,
            },
        },
    )?;
    later(game, id, 0.2, "rogue:tag_place")
}

/// Rogue tag services (`RogueTag`).
pub struct RogueTag;

impl RogueTag {
    /// Register tag-token entities (`constructor`).
    pub fn new(game: &mut Q1EntityServices) -> Result<Self, Q1Error> {
        game.named.register(
            "rogue:tag_respawn",
            Q1CallbackHandlers {
                action: Some(tag_respawn),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:tag_fall",
            Q1CallbackHandlers {
                action: Some(tag_fall),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:tag_place",
            Q1CallbackHandlers {
                action: Some(tag_place),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:tag_think",
            Q1CallbackHandlers {
                action: Some(tag_think),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:tag_touch",
            Q1CallbackHandlers {
                touch: Some(tag_touch),
                ..Default::default()
            },
        )?;
        game.register_spawn("dmatch_tag_token", spawn_token)?;
        Ok(Self)
    }

    /// Score a tag kill (`score`).
    pub fn score(
        &self,
        game: &mut Q1EntityServices,
        victim: &ActorId,
        attacker: &ActorId,
    ) -> Result<i32, Q1Error> {
        let token = token(game);
        let Some(token) = token else {
            return Ok(1);
        };
        let owner = game
            .world
            .as_ref()
            .and_then(|world| game.entity(world))
            .and_then(|world| {
                world
                    .references
                    .get("rogue:tag_token_owner")
                    .cloned()
                    .flatten()
            });
        if owner
            .as_ref()
            .is_some_and(|owner| same_actor(attacker, owner))
        {
            let frags = game
                .entity(&token)
                .map(|token| token.number("tag_frags"))
                .unwrap_or(0.0)
                + 1.0;
            game.update_entity(&token, |token| number(token, "tag_frags", frags))?;
            if frags == 5.0 {
                if game.player_ref(attacker).is_some() {
                    game.message(Some(attacker), "$qc_got_quad", false, Vec::new());
                    game.give_powerup(attacker, Q1Powerup::Quad, 30.0)?;
                }
            } else if frags == 10.0 {
                announce(game, "$qc_lost_token", attacker)?;
                respawn(game)?;
            }
            return Ok(3);
        }
        if owner
            .as_ref()
            .is_some_and(|owner| same_actor(victim, owner))
        {
            if game.host.actors.resolve_owned(victim).is_some() {
                game.sound_simple(victim, "runes/end1.wav")?;
            }
            if game.is_player(attacker) {
                take(game, &token, attacker, 0.5)?;
            }
            return Ok(5);
        }
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{
        Q1Edition, Q1FoundationOptions, Q1PrecacheProgram, Q1Trace,
    };
    use crate::q1::missionpacks::types::test_game;
    use qa_core::identity::ProviderId;

    use super::super::{MissionpackWorldHooks, install_missionpack_hooks};

    fn tag_game() -> Q1EntityServices {
        let (mut host, _) = mock_host();
        host.trace = Box::new(|request: &Q1TraceRequest| Q1Trace {
            fraction: 0.5,
            end: request.start,
            normal: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            actor: None,
            start_solid: false,
            all_solid: false,
            sky: false,
            in_open: true,
            in_water: false,
        });
        let options = Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 1,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: Some(3),
            aim_threshold: None,
        };
        Q1EntityServices::new(host, options).expect("game")
    }

    fn with_hooks(game: &mut Q1EntityServices) {
        install_missionpack_hooks(
            game,
            MissionpackWorldHooks {
                player_name: Some(Box::new(|_| "player".to_string())),
                ..Default::default()
            },
        );
    }

    fn with_player(game: &mut Q1EntityServices) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        game.set_health(&player, 100.0).expect("health");
        let watch = player.clone();
        game.host.players = Box::new(move || vec![watch.clone()]);
        player
    }

    #[test]
    fn token_spawns_and_places_on_floor() {
        let mut game = tag_game();
        RogueTag::new(&mut game).expect("tag");
        let id = game.create("dmatch_tag_token", None, None).expect("token");
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(
            game.entity(&id).expect("token").think.as_deref(),
            Some("rogue:tag_place")
        );
        game.invoke_action(&id, "rogue:tag_place").expect("place");
        assert!(game.entity(&id).is_some());
        assert_eq!(game.entity(&id).expect("token").solid, Q1Solid::Trigger);
    }

    #[test]
    fn token_touch_claims_and_scores_carrier_kills() {
        let mut game = tag_game();
        RogueTag::new(&mut game).expect("tag");
        with_hooks(&mut game);
        let world = game.create("worldspawn", None, None).expect("world");
        game.world = Some(world);
        let id = game.create("dmatch_tag_token", None, None).expect("token");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_action(&id, "rogue:tag_place").expect("place");
        let player = with_player(&mut game);
        game.invoke_touch(&id, &player, None, None).expect("touch");
        assert_eq!(
            game.entity(&id).expect("token").owner.as_ref(),
            Some(&player)
        );
        let tag = RogueTag;
        let victim = game.create("player", None, None).expect("victim");
        assert_eq!(tag.score(&mut game, &victim, &player).expect("score"), 3);
        assert_eq!(game.entity(&id).expect("token").number("tag_frags"), 1.0);
    }

    #[test]
    fn killing_carrier_passes_token() {
        let mut game = tag_game();
        RogueTag::new(&mut game).expect("tag");
        with_hooks(&mut game);
        let world = game.create("worldspawn", None, None).expect("world");
        game.world = Some(world);
        let id = game.create("dmatch_tag_token", None, None).expect("token");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_action(&id, "rogue:tag_place").expect("place");
        let carrier = with_player(&mut game);
        game.invoke_touch(&id, &carrier, None, None).expect("touch");
        let killer = game.create("player", None, None).expect("killer");
        let watch = killer.clone();
        game.host.players = Box::new(move || vec![watch.clone()]);
        let tag = RogueTag;
        assert_eq!(tag.score(&mut game, &carrier, &killer).expect("score"), 5);
        assert_eq!(
            game.entity(&id).expect("token").owner.as_ref(),
            Some(&killer)
        );
    }

    #[test]
    fn tag_disabled_without_teamplay_three() {
        let mut game = test_game();
        RogueTag::new(&mut game).expect("tag");
        let id = game.create("dmatch_tag_token", None, None).expect("token");
        game.spawn_entity(&id, None).expect("spawn");
        assert!(game.entity(&id).is_none());
    }
}
