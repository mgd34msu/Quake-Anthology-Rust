//! Runtime worldspawn precache declarations
//! (`src/content/q1/foundation/precache-world.ts`).
//!
//! Quake progs106 weapons.qc W_Precache and world.qc worldspawn.
//! GPL-2.0-or-later.

use super::entity_services::Q1EntityServices;
use crate::q1::Q1Error;

/// Runtime worldspawn declarations, in source order
/// (`precacheQ1World`). main() is compiler packaging, not a spawn
/// call.
pub fn precache_q1_world(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    for sound in [
        "weapons/r_exp3.wav",
        "weapons/rocket1i.wav",
        "weapons/sgun1.wav",
        "weapons/guncock.wav",
        "weapons/ric1.wav",
        "weapons/ric2.wav",
        "weapons/ric3.wav",
        "weapons/spike2.wav",
        "weapons/tink1.wav",
        "weapons/grenade.wav",
        "weapons/bounce.wav",
        "weapons/shotgn2.wav",
        "demon/dland2.wav",
        "misc/h2ohit1.wav",
        "items/itembk2.wav",
        "player/plyrjmp8.wav",
        "player/land.wav",
        "player/land2.wav",
        "player/drown1.wav",
        "player/drown2.wav",
        "player/gasp1.wav",
        "player/gasp2.wav",
        "player/h2odeath.wav",
        "misc/talk.wav",
        "player/teledth1.wav",
        "misc/r_tele1.wav",
        "misc/r_tele2.wav",
        "misc/r_tele3.wav",
        "misc/r_tele4.wav",
        "misc/r_tele5.wav",
        "weapons/lock4.wav",
        "weapons/pkup.wav",
        "items/armor1.wav",
        "weapons/lhit.wav",
        "weapons/lstart.wav",
        "items/damage3.wav",
        "misc/power.wav",
        "player/gib.wav",
        "player/udeath.wav",
        "player/tornoff2.wav",
        "player/pain1.wav",
        "player/pain2.wav",
        "player/pain3.wav",
        "player/pain4.wav",
        "player/pain5.wav",
        "player/pain6.wav",
        "player/death1.wav",
        "player/death2.wav",
        "player/death3.wav",
        "player/death4.wav",
        "player/death5.wav",
        "weapons/ax1.wav",
        "player/axhit1.wav",
        "player/axhit2.wav",
        "player/h2ojump.wav",
        "player/slimbrn2.wav",
        "player/inh2o.wav",
        "player/inlava.wav",
        "misc/outwater.wav",
        "player/lburn1.wav",
        "player/lburn2.wav",
        "misc/water1.wav",
        "misc/water2.wav",
    ] {
        game.precache_sound(sound)?;
    }
    for model in [
        "progs/player.mdl",
        "progs/eyes.mdl",
        "progs/h_player.mdl",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/s_bubble.spr",
        "progs/s_explod.spr",
        "progs/v_axe.mdl",
        "progs/v_shot.mdl",
        "progs/v_nail.mdl",
        "progs/v_rock.mdl",
        "progs/v_shot2.mdl",
        "progs/v_nail2.mdl",
        "progs/v_rock2.mdl",
        "progs/bolt.mdl",
        "progs/bolt2.mdl",
        "progs/bolt3.mdl",
        "progs/lavaball.mdl",
        "progs/missile.mdl",
        "progs/grenade.mdl",
        "progs/spike.mdl",
        "progs/s_spike.mdl",
        "progs/backpack.mdl",
        "progs/zom_gib.mdl",
        "progs/v_light.mdl",
    ] {
        game.precache_model(model)?;
    }
    Ok(())
}
