//! Quake III source values from `bg_public.h` and `q_shared.h`.
//!
//! Donor provenance: `src/movement/q3/constants.ts`.
//!
//! Values are `i32` donor numbers (bitwise ops, shifts, and `| 0`
//! truncation apply). The `u32` subsets in [`super::super`] (`Q3MoveType`,
//! `q3_flag`, `q3_event`) remain canonical for their existing users.

/// Player movement types.
pub mod move_type {
    /// Normal.
    pub const NORMAL: i32 = 0;
    /// Noclip.
    pub const NOCLIP: i32 = 1;
    /// Spectator.
    pub const SPECTATOR: i32 = 2;
    /// Dead.
    pub const DEAD: i32 = 3;
    /// Freeze.
    pub const FREEZE: i32 = 4;
    /// Intermission.
    pub const INTERMISSION: i32 = 5;
    /// Single-player intermission.
    pub const SPINTERMISSION: i32 = 6;
}

/// Weapon states.
pub mod weapon_state {
    /// Ready.
    pub const READY: i32 = 0;
    /// Raising.
    pub const RAISING: i32 = 1;
    /// Dropping.
    pub const DROPPING: i32 = 2;
    /// Firing.
    pub const FIRING: i32 = 3;
}

/// Powerups.
pub mod powerup {
    /// None.
    pub const NONE: i32 = 0;
    /// Quad.
    pub const QUAD: i32 = 1;
    /// Battlesuit.
    pub const BATTLESUIT: i32 = 2;
    /// Haste.
    pub const HASTE: i32 = 3;
    /// Invisibility.
    pub const INVIS: i32 = 4;
    /// Regen.
    pub const REGEN: i32 = 5;
    /// Flight.
    pub const FLIGHT: i32 = 6;
    /// Red flag.
    pub const REDFLAG: i32 = 7;
    /// Blue flag.
    pub const BLUEFLAG: i32 = 8;
    /// Neutral flag.
    pub const NEUTRALFLAG: i32 = 9;
    /// Scout.
    pub const SCOUT: i32 = 10;
    /// Guard.
    pub const GUARD: i32 = 11;
    /// Doubler.
    pub const DOUBLER: i32 = 12;
    /// Ammo regen.
    pub const AMMOREGEN: i32 = 13;
    /// Invulnerability.
    pub const INVULNERABILITY: i32 = 14;
    /// Powerup count.
    pub const NUM_POWERUPS: i32 = 15;
}

/// Holdable items.
pub mod holdable {
    /// None.
    pub const NONE: i32 = 0;
    /// Teleporter.
    pub const TELEPORTER: i32 = 1;
    /// Medkit.
    pub const MEDKIT: i32 = 2;
    /// Kamikaze.
    pub const KAMIKAZE: i32 = 3;
    /// Portal.
    pub const PORTAL: i32 = 4;
    /// Invulnerability.
    pub const INVULNERABILITY: i32 = 5;
    /// Holdable count.
    pub const NUM_HOLDABLE: i32 = 6;
}

/// Weapons.
pub mod weapon {
    /// None.
    pub const NONE: i32 = 0;
    /// Gauntlet.
    pub const GAUNTLET: i32 = 1;
    /// Machinegun.
    pub const MACHINEGUN: i32 = 2;
    /// Shotgun.
    pub const SHOTGUN: i32 = 3;
    /// Grenade launcher.
    pub const GRENADE_LAUNCHER: i32 = 4;
    /// Rocket launcher.
    pub const ROCKET_LAUNCHER: i32 = 5;
    /// Lightning gun.
    pub const LIGHTNING: i32 = 6;
    /// Railgun.
    pub const RAILGUN: i32 = 7;
    /// Plasmagun.
    pub const PLASMAGUN: i32 = 8;
    /// BFG.
    pub const BFG: i32 = 9;
    /// Grappling hook.
    pub const GRAPPLING_HOOK: i32 = 10;
    /// Nailgun.
    pub const NAILGUN: i32 = 11;
    /// Proximity launcher.
    pub const PROX_LAUNCHER: i32 = 12;
    /// Chaingun.
    pub const CHAINGUN: i32 = 13;
}

/// Entity events.
pub mod entity_event {
    /// None.
    pub const NONE: i32 = 0;
    /// Footstep.
    pub const FOOTSTEP: i32 = 1;
    /// Metal footstep.
    pub const FOOTSTEP_METAL: i32 = 2;
    /// Footsplash.
    pub const FOOTSPLASH: i32 = 3;
    /// Footwade.
    pub const FOOTWADE: i32 = 4;
    /// Swim.
    pub const SWIM: i32 = 5;
    /// Four-unit step.
    pub const STEP_4: i32 = 6;
    /// Eight-unit step.
    pub const STEP_8: i32 = 7;
    /// Twelve-unit step.
    pub const STEP_12: i32 = 8;
    /// Sixteen-unit step.
    pub const STEP_16: i32 = 9;
    /// Short fall.
    pub const FALL_SHORT: i32 = 10;
    /// Medium fall.
    pub const FALL_MEDIUM: i32 = 11;
    /// Far fall.
    pub const FALL_FAR: i32 = 12;
    /// Jump pad.
    pub const JUMP_PAD: i32 = 13;
    /// Jump.
    pub const JUMP: i32 = 14;
    /// Water touch.
    pub const WATER_TOUCH: i32 = 15;
    /// Water leave.
    pub const WATER_LEAVE: i32 = 16;
    /// Water under.
    pub const WATER_UNDER: i32 = 17;
    /// Water clear.
    pub const WATER_CLEAR: i32 = 18;
    /// Item pickup.
    pub const ITEM_PICKUP: i32 = 19;
    /// Global item pickup.
    pub const GLOBAL_ITEM_PICKUP: i32 = 20;
    /// No ammo.
    pub const NOAMMO: i32 = 21;
    /// Change weapon.
    pub const CHANGE_WEAPON: i32 = 22;
    /// Fire weapon.
    pub const FIRE_WEAPON: i32 = 23;
    /// Use item base (tag added).
    pub const USE_ITEM0: i32 = 24;
    /// Item respawn.
    pub const ITEM_RESPAWN: i32 = 40;
    /// Item pop.
    pub const ITEM_POP: i32 = 41;
    /// Teleport in.
    pub const PLAYER_TELEPORT_IN: i32 = 42;
    /// Teleport out.
    pub const PLAYER_TELEPORT_OUT: i32 = 43;
    /// Grenade bounce.
    pub const GRENADE_BOUNCE: i32 = 44;
    /// General sound.
    pub const GENERAL_SOUND: i32 = 45;
    /// Global sound.
    pub const GLOBAL_SOUND: i32 = 46;
    /// Global team sound.
    pub const GLOBAL_TEAM_SOUND: i32 = 47;
    /// Bullet hit flesh.
    pub const BULLET_HIT_FLESH: i32 = 48;
    /// Bullet hit wall.
    pub const BULLET_HIT_WALL: i32 = 49;
    /// Missile hit.
    pub const MISSILE_HIT: i32 = 50;
    /// Missile miss.
    pub const MISSILE_MISS: i32 = 51;
    /// Missile miss metal.
    pub const MISSILE_MISS_METAL: i32 = 52;
    /// Rail trail.
    pub const RAILTRAIL: i32 = 53;
    /// Shotgun.
    pub const SHOTGUN: i32 = 54;
    /// Bullet.
    pub const BULLET: i32 = 55;
    /// Pain.
    pub const PAIN: i32 = 56;
    /// Death 1.
    pub const DEATH1: i32 = 57;
    /// Death 2.
    pub const DEATH2: i32 = 58;
    /// Death 3.
    pub const DEATH3: i32 = 59;
    /// Obituary.
    pub const OBITUARY: i32 = 60;
    /// Quad powerup.
    pub const POWERUP_QUAD: i32 = 61;
    /// Battlesuit powerup.
    pub const POWERUP_BATTLESUIT: i32 = 62;
    /// Regen powerup.
    pub const POWERUP_REGEN: i32 = 63;
    /// Gib player.
    pub const GIB_PLAYER: i32 = 64;
    /// Score plum.
    pub const SCOREPLUM: i32 = 65;
    /// Proximity mine stick.
    pub const PROXIMITY_MINE_STICK: i32 = 66;
    /// Proximity mine trigger.
    pub const PROXIMITY_MINE_TRIGGER: i32 = 67;
    /// Kamikaze.
    pub const KAMIKAZE: i32 = 68;
    /// Obelisk explode.
    pub const OBELISKEXPLODE: i32 = 69;
    /// Obelisk pain.
    pub const OBELISKPAIN: i32 = 70;
    /// Invulnerability impact.
    pub const INVUL_IMPACT: i32 = 71;
    /// Juiced.
    pub const JUICED: i32 = 72;
    /// Lightning bolt.
    pub const LIGHTNINGBOLT: i32 = 73;
    /// Debug line.
    pub const DEBUG_LINE: i32 = 74;
    /// Stop looping sound.
    pub const STOPLOOPINGSOUND: i32 = 75;
    /// Taunt.
    pub const TAUNT: i32 = 76;
    /// Taunt yes.
    pub const TAUNT_YES: i32 = 77;
    /// Taunt no.
    pub const TAUNT_NO: i32 = 78;
    /// Taunt follow me.
    pub const TAUNT_FOLLOWME: i32 = 79;
    /// Taunt get flag.
    pub const TAUNT_GETFLAG: i32 = 80;
    /// Taunt guard base.
    pub const TAUNT_GUARDBASE: i32 = 81;
    /// Taunt patrol.
    pub const TAUNT_PATROL: i32 = 82;
}

/// Movement flags.
pub mod move_flags {
    /// Ducked.
    pub const DUCKED: i32 = 1;
    /// Jump held.
    pub const JUMP_HELD: i32 = 2;
    /// Backwards jump.
    pub const BACKWARDS_JUMP: i32 = 8;
    /// Backwards run.
    pub const BACKWARDS_RUN: i32 = 16;
    /// Landing timer.
    pub const TIME_LAND: i32 = 32;
    /// Knockback timer.
    pub const TIME_KNOCKBACK: i32 = 64;
    /// Water-jump timer.
    pub const TIME_WATERJUMP: i32 = 256;
    /// Respawned.
    pub const RESPAWNED: i32 = 512;
    /// Use-item held.
    pub const USE_ITEM_HELD: i32 = 1024;
    /// Grapple pull.
    pub const GRAPPLE_PULL: i32 = 2048;
    /// Follow.
    pub const FOLLOW: i32 = 4096;
    /// Scoreboard.
    pub const SCOREBOARD: i32 = 8192;
    /// Invulnerability expand.
    pub const INVULEXPAND: i32 = 16384;
}

/// Command buttons.
pub mod command_buttons {
    /// Attack.
    pub const ATTACK: i32 = 1;
    /// Talk.
    pub const TALK: i32 = 2;
    /// Use holdable.
    pub const USE_HOLDABLE: i32 = 4;
    /// Gesture.
    pub const GESTURE: i32 = 8;
    /// Walking.
    pub const WALKING: i32 = 16;
    /// Affirmative.
    pub const AFFIRMATIVE: i32 = 32;
    /// Negative.
    pub const NEGATIVE: i32 = 64;
    /// Get flag.
    pub const GETFLAG: i32 = 128;
    /// Guard base.
    pub const GUARDBASE: i32 = 256;
    /// Patrol.
    pub const PATROL: i32 = 512;
    /// Follow me.
    pub const FOLLOWME: i32 = 1024;
    /// Any.
    pub const ANY: i32 = 2048;
}

/// Player animations.
pub mod player_animation {
    /// Death 1.
    pub const BOTH_DEATH1: i32 = 0;
    /// Dead 1.
    pub const BOTH_DEAD1: i32 = 1;
    /// Death 2.
    pub const BOTH_DEATH2: i32 = 2;
    /// Dead 2.
    pub const BOTH_DEAD2: i32 = 3;
    /// Death 3.
    pub const BOTH_DEATH3: i32 = 4;
    /// Dead 3.
    pub const BOTH_DEAD3: i32 = 5;
    /// Torso gesture.
    pub const TORSO_GESTURE: i32 = 6;
    /// Torso attack.
    pub const TORSO_ATTACK: i32 = 7;
    /// Torso attack 2.
    pub const TORSO_ATTACK2: i32 = 8;
    /// Torso drop.
    pub const TORSO_DROP: i32 = 9;
    /// Torso raise.
    pub const TORSO_RAISE: i32 = 10;
    /// Torso stand.
    pub const TORSO_STAND: i32 = 11;
    /// Torso stand 2.
    pub const TORSO_STAND2: i32 = 12;
    /// Legs crouched walk.
    pub const LEGS_WALKCR: i32 = 13;
    /// Legs walk.
    pub const LEGS_WALK: i32 = 14;
    /// Legs run.
    pub const LEGS_RUN: i32 = 15;
    /// Legs back.
    pub const LEGS_BACK: i32 = 16;
    /// Legs swim.
    pub const LEGS_SWIM: i32 = 17;
    /// Legs jump.
    pub const LEGS_JUMP: i32 = 18;
    /// Legs land.
    pub const LEGS_LAND: i32 = 19;
    /// Legs backwards jump.
    pub const LEGS_JUMPB: i32 = 20;
    /// Legs backwards land.
    pub const LEGS_LANDB: i32 = 21;
    /// Legs idle.
    pub const LEGS_IDLE: i32 = 22;
    /// Legs crouched idle.
    pub const LEGS_IDLECR: i32 = 23;
    /// Legs turn.
    pub const LEGS_TURN: i32 = 24;
    /// Torso get flag.
    pub const TORSO_GETFLAG: i32 = 25;
    /// Torso guard base.
    pub const TORSO_GUARDBASE: i32 = 26;
    /// Torso patrol.
    pub const TORSO_PATROL: i32 = 27;
    /// Torso follow me.
    pub const TORSO_FOLLOWME: i32 = 28;
    /// Torso affirmative.
    pub const TORSO_AFFIRMATIVE: i32 = 29;
    /// Torso negative.
    pub const TORSO_NEGATIVE: i32 = 30;
    /// Legs backwards crouch.
    pub const LEGS_BACKCR: i32 = 32;
    /// Legs backwards walk.
    pub const LEGS_BACKWALK: i32 = 33;
    /// Flag run.
    pub const FLAG_RUN: i32 = 34;
    /// Flag stand.
    pub const FLAG_STAND: i32 = 35;
    /// Flag stand-to-run.
    pub const FLAG_STAND2RUN: i32 = 36;
}

#[cfg(test)]
mod tests {
    use super::{
        command_buttons as B, entity_event as E, holdable as H, move_flags as F, move_type as T, player_animation as A,
        powerup as P, weapon as W, weapon_state as S,
    };

    #[test]
    fn enumerations_match_bg_public() {
        assert_eq!((T::NORMAL, T::SPINTERMISSION), (0, 6));
        assert_eq!((S::READY, S::FIRING), (0, 3));
        assert_eq!((P::NONE, P::NUM_POWERUPS), (0, 15));
        assert_eq!((H::NONE, H::NUM_HOLDABLE), (0, 6));
        assert_eq!((W::NONE, W::CHAINGUN), (0, 13));
        assert_eq!((E::NONE, E::TAUNT_PATROL), (0, 82));
        assert_eq!(E::USE_ITEM0, 24);
        assert_eq!((F::DUCKED, F::INVULEXPAND), (1, 16384));
        assert_eq!((B::ATTACK, B::ANY), (1, 2048));
        assert_eq!((A::BOTH_DEATH1, A::FLAG_STAND2RUN), (0, 36));
        assert_eq!(A::LEGS_BACKCR, 32);
    }
}
