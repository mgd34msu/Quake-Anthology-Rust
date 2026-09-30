//! Q1 addon corpses (`src/content/q1/addons/corpses.ts`).
//!
//! `quakec_mg1/misc_corpses.qc` and Honey inhibition.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;

use crate::q1::addons::context::{
    addon_program, removed_for_runes, removed_outside_coop, require_entity, Q1AddonContext, Q1AddonProgram,
};
use crate::q1::base::map_entities::make_static;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1Event, Q1MoveType, Q1Solid};
use crate::q1::{q1_error, Q1Error};

/// Final corpse-frame macros from the corresponding source model frame
/// declarations: model stem plus frame.
const POSES: &[(&str, i32)] = &[
    ("demon", 53),
    ("dog", 16),
    ("dog", 25),
    ("enforcer", 54),
    ("enforcer", 65),
    ("fish", 38),
    ("hknight", 53),
    ("hknight", 62),
    ("knight", 85),
    ("knight", 96),
    ("ogre", 116),
    ("ogre", 126),
    ("shalrath", 22),
    ("shambler", 87),
    ("soldier", 17),
    ("soldier", 28),
    ("wizard", 53),
    ("player", 49),
    ("player", 60),
    ("player", 69),
    ("player", 84),
    ("player", 93),
    ("player", 102),
    ("h_demon", 0),
    ("h_dog", 0),
    ("h_guard", 0),
    ("h_hellkn", 0),
    ("h_knight", 0),
    ("h_mega", 0),
    ("h_ogre", 0),
    ("h_player", 0),
    ("h_shal", 0),
    ("h_shams", 0),
    ("h_wizard", 0),
    ("h_zombie", 0),
    ("gib1", 0),
    ("gib2", 0),
    ("gib3", 0),
];

/// Ambient classname, sound stem, and volume.
const AMBIENT: &[(&str, &str, f64)] = &[
    ("ambient_suck_wind", "suck1", 1.0),
    ("ambient_drone", "drone6", 0.5),
    ("ambient_flouro_buzz", "buzz1", 1.0),
    ("ambient_drip", "drip1", 0.5),
    ("ambient_comp_hum", "comp1", 1.0),
    ("ambient_thunder", "thunder1", 0.5),
    ("ambient_light_buzz", "fl_hum1", 0.5),
    ("ambient_swamp1", "swamp1", 0.5),
    ("ambient_swamp2", "swamp2", 0.5),
];

fn spawn_misc_corpse(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if addon_program(game)? == Q1AddonProgram::Mg3
        && (removed_outside_coop(game, id, true)? || removed_for_runes(game, id)?)
    {
        return Ok(());
    }
    let style = require_entity(game, id)?.number("style") as usize;
    let pose = POSES
        .get(style)
        .ok_or_else(|| q1_error("misc_corpse with invalid style"))?;
    game.update_entity(id, |entity| {
        entity.model = format!("progs/{}.mdl", pose.0);
        entity.frame = pose.1;
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
    })
}

fn spawn_ambient(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let classname = require_entity(game, id)?.classname.clone();
    let (_, sound, volume) = AMBIENT
        .iter()
        .find(|(name, _, _)| *name == classname)
        .ok_or_else(|| q1_error(format!("Unknown Q1 ambient classname: {classname}")))?;
    let path = game.precache_sound(&format!("ambience/{sound}.wav"))?;
    let origin = game.body(id)?.origin;
    game.host.emit(Q1Event::Ambient {
        origin,
        path,
        volume: *volume,
        attenuation: 3.0,
    });
    make_static(game, id)
}

fn spawn_ambient_generic(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let path = entity.text("noise");
    if path.is_empty() {
        return game.remove(id);
    }
    game.precache_sound(&path)?;
    let origin = game.body(id)?.origin;
    let volume = entity.number("volume");
    game.host.emit(Q1Event::Ambient {
        origin,
        path,
        volume: if volume == 0.0 { 0.5 } else { volume },
        attenuation: if entity.delay == 0.0 { 3.0 } else { entity.delay },
    });
    make_static(game, id)
}

/// Register addon corpses (`registerAddonCorpses`).
pub fn register_addon_corpses(_context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.register_spawn("misc_corpse", spawn_misc_corpse)?;
    for (classname, _, _) in AMBIENT {
        if *classname == "ambient_drone" || *classname == "ambient_comp_hum" {
            game.register_spawn(classname, spawn_ambient)?;
        } else {
            game.replace_spawn(classname, spawn_ambient)?;
        }
    }
    game.register_spawn("ambient_generic", spawn_ambient_generic)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::register_test_addons;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, program: Q1AddonProgram) -> (Q1BaseGuard, Q1AddonContext) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(game, program);
        register_addon_corpses(&context, game).expect("corpses");
        (guard, context)
    }

    #[test]
    fn corpse_poses_match_source_frames() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        assert_eq!(POSES.len(), 38);
        let corpse = game.create("misc_corpse", None, None).expect("corpse");
        spawn_misc_corpse(&mut game, &corpse).expect("spawn");
        let entity = require_entity(&game, &corpse).expect("entity");
        assert_eq!(entity.model, "progs/demon.mdl");
        assert_eq!(entity.frame, 53);

        let knight = game.create("misc_corpse", None, None).expect("knight");
        game.update_entity(&knight, |entity| {
            entity.fields.insert(String::from("style"), String::from("9"));
        })
        .expect("style");
        spawn_misc_corpse(&mut game, &knight).expect("spawn");
        let entity = require_entity(&game, &knight).expect("entity");
        assert_eq!(entity.model, "progs/knight.mdl");
        assert_eq!(entity.frame, 96);

        let invalid = game.create("misc_corpse", None, None).expect("invalid");
        game.update_entity(&invalid, |entity| {
            entity.fields.insert(String::from("style"), String::from("99"));
        })
        .expect("style");
        assert!(spawn_misc_corpse(&mut game, &invalid).is_err());
    }

    #[test]
    fn honey_inhibits_flagged_corpses() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let corpse = game.create("misc_corpse", None, None).expect("corpse");
        game.update_entity(&corpse, |entity| entity.spawnflags = 32768)
            .expect("flags");
        spawn_misc_corpse(&mut game, &corpse).expect("spawn");
        assert!(game.entity_ref(&corpse).is_none());
    }

    #[test]
    fn ambient_emitters_become_static() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        for classname in ["ambient_drone", "ambient_suck_wind", "ambient_comp_hum"] {
            let emitter = game.create(classname, None, None).expect("emitter");
            spawn_ambient(&mut game, &emitter).expect("spawn");
            assert!(game.entity_ref(&emitter).is_none(), "{classname} stays static");
        }
        let generic = game.create("ambient_generic", None, None).expect("generic");
        spawn_ambient_generic(&mut game, &generic).expect("spawn");
        assert!(game.entity_ref(&generic).is_none());

        let silent = game.create("ambient_generic", None, None).expect("silent");
        game.update_entity(&silent, |entity| {
            entity
                .fields
                .insert(String::from("noise"), String::from("ambience/wind.wav"));
        })
        .expect("noise");
        spawn_ambient_generic(&mut game, &silent).expect("spawn");
        assert!(game.entity_ref(&silent).is_none());
    }
}
