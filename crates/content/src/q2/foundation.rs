//! Q2 foundation barrel (`src/content/q2/foundation/index.ts`).
//!
//! Pure re-export barrel aggregating the foundation siblings. The donor's
//! classes map onto the arena: `Q2EntityServices`/`Q2Foundation` are the
//! [`host::Q2GameServices`] arena (`load_source` in [`runtime`] is
//! `Q2Foundation.load`), `Q2LinearMotion`/`Q2AngularMotion` are the free
//! functions in [`motion`]/[`angular_motion`], and `Q2SpawnModule` is
//! [`host::SpawnModule`].
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

pub mod angular_motion;
pub mod callbacks;
pub mod checkpoint;
pub mod effect_resources;
pub mod entity_services;
pub mod fields;
pub mod held_weapons;
pub mod host;
pub mod items;
pub mod monsters;
pub mod motion;
pub mod movers;
pub mod runtime;
pub mod scenery;
pub mod shadow_lights;
pub mod start_items;
pub mod targets;
pub mod weapon_attachments;
pub mod weapons;

pub use angular_motion::{
    angular_motion_callbacks, angular_move_to, capture_angular_motion, restore_angular_motion,
    Q2AngularMotionCheckpoint,
};
pub use callbacks::{Q2CallbackDefinitions, Q2SourceCallbacks};
pub use checkpoint::{Q2AttackCheckpoint, Q2EntityCheckpoint, Q2FoundationCheckpoint};
pub use entity_services::{delayed_use, free_q2_entity_die};
pub use fields::{inhibit_q2_spawn, parse_q2_entities};
pub use host::{
    Q2Die, Q2Entity, Q2FoundationHost, Q2GameOptions, Q2GameServices, Q2LandmarkCarry, Q2Motion, Q2Pain,
    Q2PresentationEvent, Q2SpawnFields, Q2Think, Q2Touch, Q2TraceRequest, Q2Use, SpawnModule,
};
pub use items::{
    create_q2_item_module, Q2DropOptions, Q2InventoryItem, Q2ItemDefinition, Q2ItemHooks, Q2ItemModule,
    Q2ItemsCheckpoint, Q2PickupPolicy, Q2PlayerPowerups,
};
pub use motion::{
    base_linear_motion_callbacks, capture_linear_motion, linear_motion_callbacks, linear_move_destination,
    linear_move_to, restore_linear_motion, LinearMotionScope, Q2LinearMotionCheckpoint,
};
pub use movers::{create_q2_mover_module, Q2MoverHooks, Q2MoverModule, Q2MoversCheckpoint};
pub use runtime::Q2SpawnReport;
pub use scenery::{create_q2_scenery_module, kill_q2_box, throw_q2_debris};
pub use targets::{create_q2_target_module, unrotate_q2_landmark};

#[cfg(test)]
mod tests {
    use super::*;
    use host::Q2Edition;

    #[test]
    fn parses_entities_through_barrel() {
        let parsed = parse_q2_entities(
            "{\n\"classname\" \"light\"\n}\n{\n\"classname\" \"worldspawn\"\n}",
            Q2Edition::Classic,
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].classname, "light");
        assert_eq!(parsed[1].ordinal, 1);
    }

    #[test]
    fn motion_callbacks_carry_scoped_names() {
        let linear = linear_motion_callbacks();
        assert!(linear.think.contains_key("q2:foundation/linear/Move_Done"));
        assert!(linear.think.contains_key("q2:foundation/linear/Think_AccelMove"));
        let base = base_linear_motion_callbacks();
        assert!(base.think.contains_key("q2:base/linear/Move_Done"));
        let angular = angular_motion_callbacks();
        assert!(angular.think.contains_key("q2:foundation/angular/AngleMove_Done"));
    }

    #[test]
    fn spawn_report_shape_matches_donor() {
        let report = Q2SpawnReport {
            authored: 2,
            inhibited: Vec::new(),
            removed_by_source: Vec::new(),
            spawned: Vec::new(),
            unsupported: Vec::new(),
            replaced: Vec::new(),
        };
        assert_eq!(report.authored, 2);
        assert!(report.spawned.is_empty());
    }
}
