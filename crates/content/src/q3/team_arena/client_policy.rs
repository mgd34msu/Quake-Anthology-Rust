//! Quake III team-arena: client policy.
//!
//! Donor provenance: `src/content/q3/team-arena/client-policy.ts`.

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::team_arena::mirrors::*;
use crate::q3::team_arena::movement_host::*;

// ---------------------------------------------------------------------------
// client-policy.ts
// ---------------------------------------------------------------------------

/// Spectator/inactivity policy services (`ClientPolicyContext`).
pub trait ClientPolicyHost: MovementHost {
    /// Entity pool.
    fn pool(&self) -> PoolRef;
    /// Server world.
    fn world(&self) -> WorldRef;
    /// Current time.
    fn time(&self) -> i32;
    /// Inactivity limit in seconds.
    fn inactivity_seconds(&self) -> i32;
    /// First follow target.
    fn follow1(&self) -> i32;
    /// Second follow target.
    fn follow2(&self) -> i32;
    /// Touch triggers for an entity.
    fn touch_triggers(&self, entity: &EntityRef);
    /// Cycle the follow target.
    fn follow_cycle(&self, entity: &EntityRef, direction: i32);
    /// Begin a client.
    fn client_begin(&self, client_num: usize);
    /// Drop a client.
    fn drop_client(&self, client_num: usize, reason: &str);
    /// Send a server command.
    fn send_server_command(&self, client_num: i32, text: &str);
}

pub(crate) const MASK_SPECTATOR: i32 = 1 | 0x10000;

pub(crate) const POLICY_VOTE_FLAGS: i32 = 0x4000 | 0x80000;

pub(crate) const POLICY_EF_TALK: i32 = 0x1000;

pub(crate) const POLICY_EF_FIRING: i32 = 0x100;

pub(crate) fn policy_client_of(entity: &EntityRef) -> ClientRef {
    match entity.borrow().client.clone() {
        Some(client) => client,
        None => panic!("Client policy requires a client entity"),
    }
}

pub(crate) fn policy_client_number(host: &dyn ClientPolicyHost, client: &ClientRef) -> usize {
    match host.pool().client_index(client) {
        Some(number) => number,
        None => panic!("Client policy received a client outside its entity pool"),
    }
}

/// Move a spectator client (`spectatorThink`).
pub fn spectator_think(host: &dyn ClientPolicyHost, entity: &EntityRef, command: &UserCommand) {
    let client = policy_client_of(entity);
    let follows = client.borrow().sess.spectator_state == spectator_state::FOLLOW;
    if !follows {
        {
            let mut record = client.borrow_mut();
            record.ps.pm_type = move_type::SPECTATOR;
            record.ps.speed = 400;
        }
        let movement_command = *command;
        host.move_client(
            entity,
            &movement_command,
            &ClientMovementOptions {
                trace_mask: MASK_SPECTATOR,
                fixed_msec: None,
                no_footsteps: false,
                gauntlet_hit: false,
                debug_level: 0,
            },
        );
        let origin = client.borrow().ps.origin;
        entity.borrow_mut().s.origin = origin;
        host.touch_triggers(entity);
        host.world().unlink(entity.borrow().s.number);
    }
    let mut record = client.borrow_mut();
    record.old_buttons = record.buttons;
    record.buttons = command.buttons;
    let pressed_attack =
        record.buttons & command_buttons::ATTACK != 0 && record.old_buttons & command_buttons::ATTACK == 0;
    drop(record);
    if pressed_attack {
        host.follow_cycle(entity, 1);
    }
}

/// Finish a spectator frame (`spectatorClientEndFrame`).
pub fn spectator_client_end_frame(host: &dyn ClientPolicyHost, entity: &EntityRef) {
    let client = policy_client_of(entity);
    let (state, spectate) = {
        let record = client.borrow();
        (record.sess.spectator_state, record.sess.spectator_client)
    };
    if state == spectator_state::FOLLOW {
        let mut number = spectate;
        if number == -1 {
            number = host.follow1();
        } else if number == -2 {
            number = host.follow2();
        }
        if number >= 0 {
            let followed = host.pool().client_at(number as usize);
            let record = followed.borrow();
            let eligible =
                record.pers.connected == connection_state::CONNECTED && record.sess.session_team != team::SPECTATOR;
            if eligible {
                let flags = (record.ps.e_flags & !POLICY_VOTE_FLAGS) | (client.borrow().ps.e_flags & POLICY_VOTE_FLAGS);
                let source = record.ps.clone();
                drop(record);
                let mut target = client.borrow_mut();
                target.ps.copy_from(&source, AuthorityCopy::PreserveAuthority);
                target.ps.pm_flags |= move_flags::FOLLOW;
                target.ps.e_flags = flags;
                return;
            }
            drop(record);
            if client.borrow().sess.spectator_client >= 0 {
                client.borrow_mut().sess.spectator_state = spectator_state::FREE;
                let number = policy_client_number(host, &client);
                host.client_begin(number);
            }
        }
    }
    let mut record = client.borrow_mut();
    if record.sess.spectator_state == spectator_state::SCOREBOARD {
        record.ps.pm_flags |= move_flags::SCOREBOARD;
    } else {
        record.ps.pm_flags &= !move_flags::SCOREBOARD;
    }
}

/// Drop idle clients (`clientInactivityTimer`).
pub fn client_inactivity_timer(host: &dyn ClientPolicyHost, client: &ClientRef) -> bool {
    let command = client.borrow().pers.cmd;
    if host.inactivity_seconds() == 0 {
        let mut record = client.borrow_mut();
        record.inactivity_time = host.time().wrapping_add(60_000);
        record.inactivity_warning = false;
    } else if command.forwardmove != 0
        || command.rightmove != 0
        || command.upmove != 0
        || command.buttons & command_buttons::ATTACK != 0
    {
        let mut record = client.borrow_mut();
        record.inactivity_time = host.time().wrapping_add(host.inactivity_seconds().wrapping_mul(1000));
        record.inactivity_warning = false;
    } else if !client.borrow().pers.local_client {
        let (deadline, warned) = {
            let record = client.borrow();
            (record.inactivity_time, record.inactivity_warning)
        };
        if host.time() > deadline {
            let number = policy_client_number(host, client);
            host.drop_client(number, "Dropped due to inactivity");
            return false;
        }
        if host.time() > deadline.wrapping_sub(10_000) && !warned {
            let mut record = client.borrow_mut();
            record.inactivity_warning = true;
            drop(record);
            let number = policy_client_number(host, client);
            host.send_server_command(number as i32, "cp \"Ten seconds until inactivity drop!\n\"");
        }
    }
    true
}

/// Track intermission exit input (`clientIntermissionThink`).
pub fn client_intermission_think(client: &ClientRef) {
    let mut record = client.borrow_mut();
    record.ps.e_flags &= !(POLICY_EF_TALK | POLICY_EF_FIRING);
    record.old_buttons = record.buttons;
    record.buttons = record.pers.cmd.buttons;
    if record.buttons
        & (command_buttons::ATTACK | command_buttons::USE_HOLDABLE)
        & (record.old_buttons ^ record.buttons)
        != 0
    {
        record.ready_to_exit = true;
    }
}
