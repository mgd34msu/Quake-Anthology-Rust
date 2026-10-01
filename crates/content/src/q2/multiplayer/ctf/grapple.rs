//! Q2 CTF grapple (`src/content/q2/multiplayer/ctf/grapple.ts`).

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::Vec3;

use crate::q2::equipment::ctf_grapple::{CtfGrappleSettings, Q2CtfGrappleEquipment};
use crate::q2::equipment::grapple_services::CtfGrappleState;
use crate::q2::equipment::grapple_weapon::{
    ctf_grapple_weapon_should_reset, fire_ctf_grapple_weapon, q2_ctf_grapple, q2_rerelease_ctf_grapple,
    step_ctf_grapple_weapon, CtfGrappleWeaponActions, GrappleWeaponInput,
};
use crate::q2::foundation::host::{Q2Edition, Q2GameServices};
use crate::q2::foundation::items::{Q2ItemDefinition, Q2ItemKindData};
use crate::q2::foundation::weapons::generic_frame::{project_weapon_animation, Q2GenericFrameState};
use crate::q2::foundation::weapons::player::{
    register_weapon_extension, request_weapon, weapon_generic_classic, weapon_generic_rerelease, weapon_kick,
    Q2WeaponContext, Q2WeaponExtension, Q2WeaponSelection,
};
use crate::q2::foundation::weapons::presentation::q2_weapon_recoil;
use crate::q2::foundation::weapons::types::{PrimaryHandoff, Q2WeaponDefinition, Q2WeaponState};
use crate::q2::support::contracts::{
    GrappleBinding, GrappleEdition, GrappleMechanic, GrappleSelection, SharedGrappleControl, TouchContact,
};

use super::native_grapple_hooks::native_grapple_hooks;
use super::types::Q2CtfHooks;

/// CTF grapple binding (`playerFrame` binding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2CtfGrappleBinding {
    /// Weapon slot.
    WeaponSlot,
    /// Offhand.
    Offhand,
}

/// CTF grapple damage eligibility (`new Q2CtfGrappleEquipment` damage).
fn ctf_grapple_can_damage(owner: ActorId, target: ActorId, game: &mut Q2GameServices) -> bool {
    if owner == target {
        return true;
    }
    let Some(team) = game.ctf.states.get(&owner).map(|state| state.team) else {
        return true;
    };
    if team == 0 {
        return true;
    }
    game.ctf.states.get(&target).map(|state| state.team) != Some(team)
}

/// CTF grapple settings (`new Q2CtfGrappleEquipment` tune).
fn ctf_grapple_settings(actor: ActorId, game: &mut Q2GameServices) -> CtfGrappleSettings {
    CtfGrappleSettings {
        fly_speed: 650.0,
        pull_speed: 650.0,
        damage: 10.0,
        players_collide: game
            .weapons
            .inputs
            .get(&actor)
            .map(|input| input.players_collide)
            .unwrap_or(true),
    }
}

/// Build CTF grapple equipment for a shared selection (`new Q2CtfGrapple`).
pub fn ctf_grapple_equipment(shared: Option<&dyn SharedGrappleControl>) -> Option<Q2CtfGrappleEquipment> {
    let native = match shared {
        None => true,
        Some(shared) => match shared.selection() {
            GrappleSelection::Enabled { binding, mechanic, .. } => {
                *binding == GrappleBinding::Slot
                    && *mechanic == GrappleMechanic::Q2Ctf
                    && shared.native_slot(GrappleMechanic::Q2Ctf)
            }
            GrappleSelection::Disabled => false,
        },
    };
    if !native {
        return None;
    }
    Some(Q2CtfGrappleEquipment {
        hooks: native_grapple_hooks(true),
        can_damage: ctf_grapple_can_damage,
        settings: ctf_grapple_settings,
    })
}

/// Grapple item pickup (`register` pickup).
fn ctf_grapple_item_pickup(_entity: ActorId, _game: &mut Q2GameServices, _player: OwnedActor) -> bool {
    false
}

/// Grapple item use (`register` use).
fn ctf_grapple_item_use(actor: OwnedActor, game: &mut Q2GameServices) -> bool {
    let native = game.ctf.equipment.is_some();
    let live = game.entity(actor.id()).is_some();
    native && live && request_weapon(game, &actor, "grapple", false) == Q2WeaponSelection::Selected
}

/// CTF grapple (`Q2CtfGrapple`).
#[derive(Debug, Clone, Copy)]
pub struct Q2CtfGrapple {
    /// Session hooks.
    pub hooks: Q2CtfHooks,
}

impl Q2CtfGrapple {
    /// Whether native grapple is enabled (`nativeEnabled`).
    pub fn native_enabled(&self, game: &Q2GameServices) -> bool {
        game.ctf.equipment.is_some()
    }

    /// Read owner grapple state (`state`).
    pub fn state(&self, game: &mut Q2GameServices, actor: ActorId) -> CtfGrappleState {
        if let Some(equipment) = game.ctf.equipment {
            equipment.state_snapshot(game, actor)
        } else {
            game.ctf.inactive_grapple.get(&actor).cloned().unwrap_or_default()
        }
    }

    /// Reset a grapple (`reset`).
    pub fn reset(&self, player: ActorId, game: &mut Q2GameServices) {
        if let Some(equipment) = game.ctf.equipment {
            equipment.reset(player, game);
        } else if let Some(shared) = game.ctf.shared.as_mut() {
            shared.release(&player);
        }
    }

    /// Run offhand input (`offhand`).
    pub fn offhand(&self, player: ActorId, game: &mut Q2GameServices, pressed: bool) {
        if let Some(equipment) = game.ctf.equipment {
            equipment.offhand(player, game, pressed);
        } else {
            self.command(player, game, pressed);
        }
    }

    /// Touch a hook (`touch`).
    pub fn touch(&self, hook: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
        if let Some(equipment) = game.ctf.equipment {
            equipment.touch(hook, game, contact);
        }
    }

    /// Fire a grapple (`fireGrapple`).
    #[allow(clippy::too_many_arguments)]
    pub fn fire_grapple(
        &self,
        player: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        effects: i64,
    ) -> bool {
        if let Some(equipment) = game.ctf.equipment {
            equipment.fire_grapple(player, game, start, direction, damage, speed, effects)
        } else {
            false
        }
    }

    /// Run the player frame (`playerFrame`).
    pub fn player_frame(&self, player: ActorId, game: &mut Q2GameServices, binding: Q2CtfGrappleBinding) {
        let weapon = game.weapons.states.get(&player).cloned();
        if let Some(weapon) = weapon {
            let selected = binding == Q2CtfGrappleBinding::WeaponSlot && weapon.weapon.as_deref() == Some("grapple");
            let holster = game
                .weapons
                .inputs
                .get(&player)
                .map(|input| input.holster)
                .unwrap_or(false);
            let change = weapon.pending.is_some()
                || weapon.primary_handoff == PrimaryHandoff::Holstering
                || game.options.edition == Q2Edition::Rerelease && holster;
            if ctf_grapple_weapon_should_reset(&project_weapon_animation(&weapon), selected, change) {
                if game.options.edition == Q2Edition::Rerelease && weapon.primary_handoff == PrimaryHandoff::Active {
                    if let Some(state) = game.weapons.states.get_mut(&player) {
                        state.pending = state.weapon.clone();
                    }
                }
                self.reset(player, game);
                return;
            }
        }
        if let Some(equipment) = game.ctf.equipment {
            equipment.player_frame(player, game, true);
        }
    }

    /// Feed the shared command (`command`).
    pub fn command(&self, player: ActorId, game: &mut Q2GameServices, pressed: bool) {
        let offhand = matches!(
            game.ctf.shared.as_ref().map(|shared| shared.selection()),
            Some(GrappleSelection::Enabled {
                binding: GrappleBinding::Offhand,
                ..
            })
        );
        if offhand {
            if let Some(shared) = game.ctf.shared.as_mut() {
                shared.input(&player, pressed);
            }
        }
    }

    /// Register the grapple weapon and item (`register`).
    pub fn register(&self, game: &mut Q2GameServices) {
        if game.ctf.equipment.is_some() {
            let rerelease = matches!(
                game.ctf.shared.as_ref().map(|shared| shared.selection()),
                Some(GrappleSelection::Enabled {
                    edition: GrappleEdition::Rerelease,
                    ..
                })
            );
            register_weapon_extension(
                game,
                Box::new(Q2CtfGrappleExtension {
                    hooks: self.hooks,
                    definition: if rerelease {
                        q2_rerelease_ctf_grapple()
                    } else {
                        q2_ctf_grapple()
                    },
                }),
            );
        }
        self.hooks.items.register_item(
            game,
            Q2ItemDefinition {
                classname: "weapon_grapple".to_string(),
                model: String::new(),
                icon: "w_grapple".to_string(),
                name: "Grapple".to_string(),
                sound: "misc/w_pkup.wav".to_string(),
                rotate: false,
                respawn: 0.0,
                console_give: None,
                kind: Q2ItemKindData::Custom {
                    capacity: 1.0,
                    quantity: 0.0,
                    coop_stay: true,
                    droppable: false,
                    pickup: ctf_grapple_item_pickup,
                    use_item: Some(ctf_grapple_item_use),
                },
            },
        );
    }
}

/// CTF grapple weapon extension (`register` weapon).
pub struct Q2CtfGrappleExtension {
    /// Session hooks.
    pub hooks: Q2CtfHooks,
    /// Weapon definition.
    pub definition: Q2WeaponDefinition,
}

/// CTF grapple think driver (`weaponFrame` actions).
struct CtfGrappleThinkDriver<'a> {
    /// Owning actor.
    owner: ActorId,
    /// Game services.
    game: &'a mut Q2GameServices,
    /// Weapon state.
    state: &'a mut Q2WeaponState,
    /// Weapon context.
    context: &'a Q2WeaponContext,
    /// Edition.
    edition: Q2Edition,
    /// Equipment.
    equipment: Q2CtfGrappleEquipment,
}

impl CtfGrappleWeaponActions for CtfGrappleThinkDriver<'_> {
    fn reset(&mut self) {
        self.equipment.reset(self.owner.clone(), self.game);
    }

    fn prepare_drop(&mut self) {
        if self.edition == Q2Edition::Rerelease
            && self.state.primary_handoff == PrimaryHandoff::Active
            && self.state.pending.is_none()
        {
            self.state.pending = self.state.weapon.clone();
        }
    }

    fn generic(&mut self, animation: &mut Q2GenericFrameState) {
        self.state.phase = animation.phase;
        self.state.frame = animation.frame;
        if self.edition == Q2Edition::Rerelease {
            weapon_generic_rerelease(self.context, self.game, self.state);
        } else {
            weapon_generic_classic(self.context, self.game, self.state);
        }
        animation.phase = self.state.phase;
        animation.frame = self.state.frame;
    }
}

impl Q2WeaponExtension for Q2CtfGrappleExtension {
    fn definition(&self) -> &Q2WeaponDefinition {
        &self.definition
    }

    fn fire(&mut self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        let Some(equipment) = game.ctf.equipment else {
            return;
        };
        let mut animation = project_weapon_animation(state);
        let owner = context.owner.actor.id().clone();
        let edition = game.options.edition;
        if let Some((origin, pitch)) = fire_ctf_grapple_weapon(owner, game, equipment, &mut animation, edition) {
            let (_, kick_angles) = q2_weapon_recoil(state, edition, context.now);
            weapon_kick(
                context,
                game,
                state,
                origin,
                Vec3 {
                    x: pitch as f32,
                    ..kick_angles
                },
            );
        }
        state.phase = animation.phase;
        state.frame = animation.frame;
    }

    fn think(&mut self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) -> bool {
        let Some(equipment) = game.ctf.equipment else {
            return true;
        };
        let edition = game.options.edition;
        let rerelease = edition == Q2Edition::Rerelease;
        let owner = context.owner.actor.id().clone();
        let context = Q2WeaponContext {
            rerelease,
            definition: if rerelease {
                q2_rerelease_ctf_grapple()
            } else {
                q2_ctf_grapple()
            },
            ..context.clone()
        };
        let input = GrappleWeaponInput {
            attack: context.input.attack,
            change_requested: state.pending.is_some() || state.primary_handoff == PrimaryHandoff::Holstering,
            holster: context.input.holster,
            latched_holster: false,
        };
        let mut animation = project_weapon_animation(state);
        let source = equipment.state_snapshot(game, owner.clone());
        let mut actions = CtfGrappleThinkDriver {
            owner,
            game,
            state: &mut *state,
            context: &context,
            edition,
            equipment,
        };
        step_ctf_grapple_weapon(&mut animation, &source, &input, edition, &mut actions);
        state.phase = animation.phase;
        state.frame = animation.frame;
        true
    }
}
