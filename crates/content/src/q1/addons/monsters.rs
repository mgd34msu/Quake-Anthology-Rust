//! Q1 addon monsters root (`src/content/q1/addons/monsters`, barrel `index.ts`).

pub mod ai;
pub mod bosses;
pub mod demodog;
pub mod demodog_frames;
pub mod heavy;
pub mod infected;
pub mod ordinary;
pub mod startup;

use crate::q1::addons::context::{Q1AddonContext, Q1AddonProgram};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::Q1Error;

/// Register addon monsters (`registerAddonMonsters`). Native maps and
/// selected foreign rosters install the same named source controllers.
pub fn register_addon_monsters(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    ordinary::register_ordinary_addon_monsters(context, game)?;
    if context.program() == Q1AddonProgram::Mg3 {
        ai::targets::register_mg3_path_targets(context, game)?;
        demodog::register_mg3_demodog(context, game)?;
        infected::register_mg3_infected(context, game)?;
        heavy::register_mg3_heavy(context, game)?;
        bosses::sacrifice::register_sacrifice(game)?;
        bosses::ghost::register_ghost(game)?;
        bosses::orb::register_orb(game)?;
        bosses::szombie::register_shub_zombie(game)?;
        bosses::oldnew::register_oldnew(game)?;
        bosses::r#final::register_final_boss(game)?;
    }
    Ok(())
}
