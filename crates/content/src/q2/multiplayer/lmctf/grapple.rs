//! Q2 LMCTF grapple (`src/content/q2/multiplayer/lmctf/grapple.ts`).

use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::Vec3;

use crate::q2::equipment::grapple_services::LmctfGrappleState;
use crate::q2::equipment::grapple_weapon::{
    GrappleWeaponInput, LmctfGrappleWeaponActions, fire_lmctf_grapple_weapon, lmctf_grapple, release_lmctf_grapple_weapon,
    step_lmctf_grapple_weapon,
};
use crate::q2::equipment::lmctf_grapple::{LmctfGrappleEquipment, LmctfGrapplePolicy, lmctf_grapple_callbacks};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::Q2GameServices;
use crate::q2::foundation::items::{Q2ItemDefinition, Q2ItemKindData};
use crate::q2::foundation::weapons::generic_frame::{Q2GenericFrameState, project_weapon_animation};
use crate::q2::foundation::weapons::player::{
    Q2WeaponContext, Q2WeaponExtension, register_weapon_extension, request_weapon, weapon_generic_classic,
};
use crate::q2::foundation::weapons::presentation::{q2_weapon_recoil, set_q2_weapon_recoil};
use crate::q2::foundation::weapons::types::{PrimaryHandoff, Q2WeaponDefinition, Q2WeaponState};
use crate::q2::support::contracts::{GrappleBinding, GrappleMechanic, GrappleSelection, SharedGrappleControl};

use super::super::ctf::native_grapple_hooks::native_grapple_hooks;
use super::super::ctf::types::item_id;
use super::types::{LmctfHooks, lmctf_active, lmctf_print};

/// Re-export the hook definition (`hookDefinition`).
pub use crate::q2::equipment::grapple_weapon::lmctf_grapple as hook_definition;

/// LMCTF grapple attach eligibility (`new LmctfGrappleEquipment` attach).
fn lmctf_grapple_can_attach(owner: ActorId, target: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.entity(&target).map(|entity| entity.classname.clone()).unwrap_or_default();
    if !game.host.is_player(&target)
        && classname != "bodyque"
        && classname != "worldspawn"
        && !classname.starts_with("func")
        && !classname.starts_with("info_flag")
        && target != game.host.world_actor()
    {
        return false;
    }
    let Some(team) = game.lmctf.states.get(&owner).map(|state| state.team) else {
        return true;
    };
    if team == 0 {
        return true;
    }
    game.lmctf.states.get(&target).map(|state| state.team) != Some(team)
}

/// LMCTF grapple damage eligibility (`new LmctfGrappleEquipment` damage).
fn lmctf_grapple_can_damage(target: ActorId, game: &mut Q2GameServices) -> bool {
    if game.lmctf.rules.ctf_flags & 64 == 0 {
        return true;
    }
    let hooks = super::lmctf_hooks(game);
    (hooks.player)(target, game).is_none()
}

/// LMCTF grapple player hits (`new LmctfGrappleEquipment` hits).
fn lmctf_grapple_player_hit(target: ActorId, game: &mut Q2GameServices) -> bool {
    lmctf_active(game, &target)
}

/// Release a hook weapon frame (`releaseLmctfGrappleWeapon` bridge).
fn release_hook_weapon(state: &mut Q2WeaponState) {
    let mut animation = project_weapon_animation(state);
    release_lmctf_grapple_weapon(&mut animation);
    state.phase = animation.phase;
    state.frame = animation.frame;
}

/// LMCTF grapple release (`new LmctfGrappleEquipment` release).
fn lmctf_grapple_released(actor: ActorId, game: &mut Q2GameServices) {
    if game.weapons.states.get(&actor).is_some_and(|state| state.weapon.as_deref() == Some("lmctf:hook")) {
        if let Some(state) = game.weapons.states.get_mut(&actor) {
            release_hook_weapon(state);
        }
    }
}

/// Build LMCTF grapple equipment for a shared selection (`new LmctfGrapple`).
pub fn lmctf_grapple_equipment(shared: Option<&dyn SharedGrappleControl>) -> Option<LmctfGrappleEquipment> {
    let native = match shared {
        None => true,
        Some(shared) => match shared.selection() {
            GrappleSelection::Enabled { binding, mechanic, .. } => {
                *binding == GrappleBinding::Slot && *mechanic == GrappleMechanic::Q2Lmctf && shared.native_slot(GrappleMechanic::Q2Lmctf)
            }
            GrappleSelection::Disabled => false,
        },
    };
    if !native {
        return None;
    }
    Some(LmctfGrappleEquipment {
        hooks: native_grapple_hooks(false),
        policy: LmctfGrapplePolicy {
            can_attach: lmctf_grapple_can_attach,
            can_damage: lmctf_grapple_can_damage,
            player_hit: lmctf_grapple_player_hit,
        },
        released: lmctf_grapple_released,
    })
}

/// LMCTF grapple (`LmctfGrapple`).
#[derive(Debug, Clone, Copy)]
pub struct LmctfGrapple {
    /// Session hooks.
    pub hooks: LmctfHooks,
}

impl LmctfGrapple {
    /// Active grapple states (`states`).
    pub fn states(game: &mut Q2GameServices) -> &mut HashMap<ActorId, LmctfGrappleState> {
        if game.lmctf.equipment.is_some() { &mut game.equipment.lmctf_states } else { &mut game.lmctf.inactive_grapple }
    }

    /// Whether native grapple is enabled (`nativeEnabled`).
    pub fn native_enabled(&self, game: &Q2GameServices) -> bool {
        game.lmctf.equipment.is_some()
    }

    /// Read owner grapple state (`state`).
    pub fn state(&self, game: &mut Q2GameServices, actor: ActorId) -> LmctfGrappleState {
        if let Some(equipment) = game.lmctf.equipment {
            equipment.state_snapshot(game, actor)
        } else {
            game.lmctf.inactive_grapple.get(&actor).cloned().unwrap_or_default()
        }
    }

    /// Read grapple callbacks (`callbacks`).
    pub fn callbacks(&self, game: &Q2GameServices) -> Q2CallbackDefinitions {
        if game.lmctf.equipment.is_some() { lmctf_grapple_callbacks() } else { Q2CallbackDefinitions::default() }
    }

    /// Read the gravity scale (`gravityScale`).
    pub fn gravity_scale(&self, actor: ActorId, game: &Q2GameServices) -> i32 {
        game.lmctf.equipment.map(|equipment| equipment.gravity_scale(game, actor)).unwrap_or(1)
    }

    /// Abort a grapple (`abort`).
    pub fn abort(&self, player: ActorId, game: &mut Q2GameServices) {
        if let Some(equipment) = game.lmctf.equipment {
            equipment.abort(player, game);
        } else if let Some(shared) = game.lmctf.shared.as_mut() {
            shared.release(&player);
        }
    }

    /// Fire a grapple (`fire`).
    pub fn fire(&self, player: ActorId, game: &mut Q2GameServices) {
        let Some(equipment) = game.lmctf.equipment else {
            return;
        };
        if !game.weapons.states.contains_key(&player) {
            equipment.fire(player, game);
            return;
        }
        let edition = game.options.edition;
        let now = game.now();
        let mut animation = project_weapon_animation(game.weapons.states.get(&player).expect("LMCTF hook weapon state is missing"));
        let kick = fire_lmctf_grapple_weapon(player.clone(), game, equipment, &mut animation);
        if let Some(state) = game.weapons.states.get_mut(&player) {
            state.phase = animation.phase;
            state.frame = animation.frame;
            if let Some((origin, pitch)) = kick {
                let (_, kick_angles) = q2_weapon_recoil(state, edition, now);
                set_q2_weapon_recoil(state, edition, now, origin, Vec3 { x: pitch as f32, ..kick_angles }, None);
            }
        }
    }

    /// Run a grapple command (`command`).
    pub fn command(&self, player: ActorId, game: &mut Q2GameServices, pressed: bool) {
        if game.lmctf.equipment.is_none() {
            let offhand = matches!(
                game.lmctf.shared.as_ref().map(|shared| shared.selection()),
                Some(GrappleSelection::Enabled { binding: GrappleBinding::Offhand, .. })
            );
            if offhand {
                if let Some(shared) = game.lmctf.shared.as_mut() {
                    shared.input(&player, pressed);
                }
            }
            return;
        }
        let blocked = match (self.hooks.player)(player.clone(), game) {
            Some(player) => player.noclip || player.spectator,
            None => true,
        };
        if blocked {
            return;
        }
        if game.lmctf.rules.ctf_flags & 16 != 0 {
            let selected = game.weapons.states.get(&player).is_some_and(|state| state.weapon.as_deref() == Some("lmctf:hook"));
            if selected {
                Self::states(game).entry(player.clone()).or_default().hook_held = pressed;
                if pressed {
                    if let Some(weapon) = game.weapons.states.get_mut(&player) {
                        weapon.latched_attack = true;
                    }
                } else {
                    self.abort(player, game);
                }
                return;
            }
            if !pressed {
                self.abort(player, game);
                return;
            }
            if Self::states(game).get(&player).and_then(|state| state.hook.clone()).is_some() {
                return;
            }
            if game.host.inventory().count(&player, &item_id("q2:weapon_hook")) == 0.0 {
                lmctf_print(game, "You have no hook.\n", Some(player));
                return;
            }
            if game.weapons.inputs.get(&player).map(|input| input.quad_until).unwrap_or(0.0) > game.now() {
                game.sound(&player, "items/damage3.wav", 3, 1.0, 1.0);
            }
            self.fire(player, game);
            return;
        }
        if pressed {
            let owned = game.owned_of(player);
            request_weapon(game, &owned, "lmctf:hook", false);
        }
    }

    /// Register the hook weapon and item (`register`).
    pub fn register(&self, game: &mut Q2GameServices) {
        if game.lmctf.equipment.is_none() {
            self.hooks.items.register_item(
                game,
                Q2ItemDefinition {
                    classname: "weapon_hook".to_string(),
                    model: String::new(),
                    icon: "w_blaster".to_string(),
                    name: "Grappling Hook".to_string(),
                    sound: "misc/w_pkup.wav".to_string(),
                    rotate: false,
                    respawn: 0.0,
                    console_give: None,
                    kind: Q2ItemKindData::Custom {
                        capacity: 1.0,
                        quantity: 0.0,
                        coop_stay: true,
                        droppable: false,
                        pickup: lmctf_hook_item_pickup,
                        use_item: Some(lmctf_hook_item_use),
                    },
                },
            );
            return;
        }
        let definition = lmctf_grapple();
        self.hooks.items.register_item(
            game,
            Q2ItemDefinition {
                classname: "weapon_hook".to_string(),
                model: definition.world_model.clone(),
                icon: "w_blaster".to_string(),
                name: "Grappling Hook".to_string(),
                sound: "misc/w_pkup.wav".to_string(),
                rotate: false,
                respawn: 0.0,
                console_give: None,
                kind: Q2ItemKindData::Weapon { ammo: None, coop_stay: Some(true) },
            },
        );
        register_weapon_extension(game, Box::new(LmctfGrappleExtension { hooks: self.hooks, definition }));
    }
}

/// Hook item pickup (`register` pickup).
fn lmctf_hook_item_pickup(_entity: ActorId, _game: &mut Q2GameServices, _player: OwnedActor) -> bool {
    false
}

/// Hook item use (`register` use).
fn lmctf_hook_item_use(_player: OwnedActor, _game: &mut Q2GameServices) -> bool {
    false
}

/// LMCTF hook weapon extension (`register` weapon).
pub struct LmctfGrappleExtension {
    /// Session hooks.
    pub hooks: LmctfHooks,
    /// Weapon definition.
    pub definition: Q2WeaponDefinition,
}

/// LMCTF hook think driver (`think` actions).
struct LmctfHookThinkDriver<'a> {
    /// Owning actor.
    owner: ActorId,
    /// Game services.
    game: &'a mut Q2GameServices,
    /// Weapon state.
    state: &'a mut Q2WeaponState,
    /// Weapon context.
    context: &'a Q2WeaponContext,
    /// Equipment.
    equipment: LmctfGrappleEquipment,
}

impl LmctfGrappleWeaponActions for LmctfHookThinkDriver<'_> {
    fn abort(&mut self) {
        self.equipment.abort(self.owner.clone(), self.game);
    }

    fn generic(&mut self, animation: &mut Q2GenericFrameState) {
        self.state.phase = animation.phase;
        self.state.frame = animation.frame;
        weapon_generic_classic(self.context, self.game, self.state);
        animation.phase = self.state.phase;
        animation.frame = self.state.frame;
    }
}

impl Q2WeaponExtension for LmctfGrappleExtension {
    fn definition(&self) -> &Q2WeaponDefinition {
        &self.definition
    }

    fn fire(&mut self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        let Some(equipment) = game.lmctf.equipment else {
            return;
        };
        let mut animation = project_weapon_animation(state);
        let owner = context.owner.actor.id().clone();
        let edition = game.options.edition;
        let now = game.now();
        let kick = fire_lmctf_grapple_weapon(owner, game, equipment, &mut animation);
        state.phase = animation.phase;
        state.frame = animation.frame;
        if let Some((origin, pitch)) = kick {
            let (_, kick_angles) = q2_weapon_recoil(state, edition, now);
            set_q2_weapon_recoil(state, edition, now, origin, Vec3 { x: pitch as f32, ..kick_angles }, None);
        }
    }

    fn think(&mut self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) -> bool {
        let Some(equipment) = game.lmctf.equipment else {
            return true;
        };
        let owner = context.owner.actor.id().clone();
        let source = equipment.state_snapshot(game, owner.clone());
        let input = GrappleWeaponInput {
            attack: context.input.attack,
            change_requested: state.pending.is_some() || state.primary_handoff == PrimaryHandoff::Holstering,
            holster: context.input.holster,
            latched_holster: false,
        };
        let mut animation = project_weapon_animation(state);
        let mut actions = LmctfHookThinkDriver { owner, game, state: &mut *state, context, equipment };
        step_lmctf_grapple_weapon(&mut animation, &source, &input, &mut actions);
        state.phase = animation.phase;
        state.frame = animation.frame;
        true
    }
}
