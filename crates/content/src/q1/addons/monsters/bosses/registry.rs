//! Q1 mg3 boss registration (`src/content/q1/addons/monsters/bosses/registry.ts`).
//!
//! Shared boss driver wiring. Controllers persist through the base
//! creature store; the extension only carries the clone hook.

use qa_core::identity::ActorId;

use crate::q1::addons::monsters::ai::{clone_monster_controller, register_mg3_monster_callbacks};
use crate::q1::addons::monsters::startup::register_mg3_monster_startup;
use crate::q1::foundation::callbacks::Q1StateExtension;
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity_services::{Q1EntityServices, Q1SpawnHandler};
use crate::q1::Q1Error;

/// Register boss driver wiring for one boss source
/// (`registerBossControllers`). The source itself must already be
/// registered.
pub fn register_boss_controllers(
    game: &mut Q1EntityServices,
    prefix: &'static str,
    classname: &str,
    spawn: Q1SpawnHandler,
) -> Result<(), Q1Error> {
    struct Extension {
        prefix: &'static str,
    }

    impl Q1StateExtension for Extension {
        fn id(&self) -> &str {
            self.prefix
        }

        fn capture(&self, _game: &Q1EntityServices) -> Vec<u8> {
            encode_checkpoint_value(&crate::value::arr(Vec::new()))
        }

        fn restore(&mut self, _game: &mut Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
            let saved = decode_checkpoint_value(bytes)?;
            crate::value::SaveReader::new(&saved).list(|_entry| Ok::<(), Q1Error>(()))?;
            Ok(())
        }

        fn clone_state(
            &mut self,
            game: &mut Q1EntityServices,
            source: &ActorId,
            target: &ActorId,
        ) -> Result<(), Q1Error> {
            clone_monster_controller(game, source, target, self.prefix)
        }
    }

    register_mg3_monster_callbacks(game, prefix)?;
    register_mg3_monster_startup(game, prefix)?;
    game.register_spawn(classname, spawn)?;
    game.register_state_extension(Box::new(Extension { prefix }))?;
    Ok(())
}
