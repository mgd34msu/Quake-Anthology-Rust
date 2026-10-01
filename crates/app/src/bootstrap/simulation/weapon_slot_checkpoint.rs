//! Weapon slot and grapple animation checkpoint readers.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/weapon-slot-checkpoint.ts`
//! (`readWeaponSlotState`, `readWeaponSlots`, `readGrappleWeaponState`).

use qa_content::q2::equipment::grapple_weapon::{GrappleHandoff, GrappleWeaponState};
use qa_content::q2::foundation::weapons::generic_frame::Q2GenericFrameState;
use qa_content::q2::foundation::weapons::types::Q2WeaponPhase;
use qa_core::identity::{ProviderId, SavedActorId};
use qa_world::save::records::read_saved_actor;
use qa_world::save::value::{namespaced, SaveReader};
use qa_world::WorldError;

use super::weapon_slot::{WeaponReference, WeaponSlotRestoreState, WeaponSlotState};

fn read_provider(reader: SaveReader) -> Result<ProviderId, WorldError> {
    let value = namespaced(reader.clone())?;
    let Some(colon) = value.find(':') else {
        return Err(reader.fail("expected a namespaced identity"));
    };
    Ok(ProviderId::new(&value[..colon], &value[colon + 1..]))
}

fn read_weapon_reference(reader: SaveReader) -> Result<WeaponReference, WorldError> {
    Ok(WeaponReference {
        provider: read_provider(reader.field("provider"))?,
        item: namespaced(reader.field("item"))?,
    })
}

/// Read one restorable weapon slot state.
pub fn read_weapon_slot_state(reader: SaveReader) -> Result<WeaponSlotRestoreState, WorldError> {
    let kind = reader.field("kind").choice_str(&[
        "active",
        "switching",
        "activating",
        "primary",
        "equipment",
        "holstering-primary",
        "holstering-equipment",
    ])?;
    match kind.as_str() {
        "active" => Ok(WeaponSlotRestoreState::State(WeaponSlotState::Active {
            provider: read_provider(reader.field("provider"))?,
        })),
        "switching" => Ok(WeaponSlotRestoreState::State(WeaponSlotState::Switching {
            from: read_provider(reader.field("from"))?,
            next: read_weapon_reference(reader.field("next"))?,
        })),
        "activating" => Ok(WeaponSlotRestoreState::State(WeaponSlotState::Activating {
            from: read_provider(reader.field("from"))?,
            next: read_weapon_reference(reader.field("next"))?,
            request: reader.field("request").integer(1)? as u64,
        })),
        "primary" => Ok(WeaponSlotRestoreState::Primary),
        "equipment" => Ok(WeaponSlotRestoreState::Equipment),
        "holstering-primary" => Ok(WeaponSlotRestoreState::HolsteringPrimary {
            next: read_weapon_reference(reader.field("next"))?,
        }),
        _ => Ok(WeaponSlotRestoreState::HolsteringEquipment {
            next: read_weapon_reference(reader.field("next"))?,
        }),
    }
}

/// One saved weapon slot entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponSlotEntry {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Restorable state.
    pub state: WeaponSlotRestoreState,
}

/// Read saved weapon slot entries.
pub fn read_weapon_slots(reader: SaveReader) -> Result<Vec<WeaponSlotEntry>, WorldError> {
    reader.list(|entry| {
        Ok(WeaponSlotEntry {
            actor: read_saved_actor(entry.field("actor"))?,
            state: read_weapon_slot_state(entry.field("state"))?,
        })
    })
}

/// Read one grapple weapon animation state.
pub fn read_grapple_weapon_state(reader: SaveReader) -> Result<GrappleWeaponState, WorldError> {
    let state = reader.field("animation");
    let phase = state
        .field("phase")
        .choice_str(&["activating", "ready", "firing", "dropping"])?;
    let handoff = reader
        .field("handoff")
        .choice_str(&["active", "holstering", "holstered"])?;
    Ok(GrappleWeaponState {
        handoff: match handoff.as_str() {
            "active" => GrappleHandoff::Active,
            "holstering" => GrappleHandoff::Holstering,
            _ => GrappleHandoff::Holstered,
        },
        animation: Q2GenericFrameState {
            phase: match phase.as_str() {
                "activating" => Q2WeaponPhase::Activating,
                "ready" => Q2WeaponPhase::Ready,
                "firing" => Q2WeaponPhase::Firing,
                _ => Q2WeaponPhase::Dropping,
            },
            frame: state.field("frame").integer(0)? as i32,
            latched_attack: state.field("latchedAttack").boolean()?,
            source_firing: state.field("sourceFiring").boolean()?,
            think_time: state.field("thinkTime").finite()?,
            fire_finished: state.field("fireFinished").finite()?,
            fire_buffered: state.field("fireBuffered").boolean()?,
            last_firing_time: state.field("lastFiringTime").finite()?,
        },
    })
}

#[cfg(test)]
mod tests {
    use qa_world::save::records::write_saved_actor;
    use qa_world::save::value::{arr, boolean, int, num, obj, str as save_str, SaveJson};

    use super::*;

    fn reference(provider: &str, item: &str) -> SaveJson {
        obj(vec![("provider", save_str(provider)), ("item", save_str(item))])
    }

    #[test]
    fn reads_all_slot_kinds() {
        let active = obj(vec![("kind", save_str("active")), ("provider", save_str("q2:base"))]);
        assert_eq!(
            read_weapon_slot_state(SaveReader::new(&active)).expect("active"),
            WeaponSlotRestoreState::State(WeaponSlotState::Active {
                provider: ProviderId::new("q2", "base"),
            })
        );
        let switching = obj(vec![
            ("kind", save_str("switching")),
            ("from", save_str("q2:base")),
            ("next", reference("q2:ctf", "q2:weapon_grapple")),
        ]);
        assert_eq!(
            read_weapon_slot_state(SaveReader::new(&switching)).expect("switching"),
            WeaponSlotRestoreState::State(WeaponSlotState::Switching {
                from: ProviderId::new("q2", "base"),
                next: WeaponReference {
                    provider: ProviderId::new("q2", "ctf"),
                    item: "q2:weapon_grapple".to_string(),
                },
            })
        );
        let activating = obj(vec![
            ("kind", save_str("activating")),
            ("from", save_str("q2:base")),
            ("next", reference("q2:ctf", "q2:weapon_grapple")),
            ("request", int(9)),
        ]);
        assert!(matches!(
            read_weapon_slot_state(SaveReader::new(&activating)).expect("activating"),
            WeaponSlotRestoreState::State(WeaponSlotState::Activating { request: 9, .. })
        ));
        for (kind, expected) in [
            ("primary", WeaponSlotRestoreState::Primary),
            ("equipment", WeaponSlotRestoreState::Equipment),
        ] {
            let json = obj(vec![("kind", save_str(kind))]);
            assert_eq!(read_weapon_slot_state(SaveReader::new(&json)).expect(kind), expected);
        }
        let holstering = obj(vec![
            ("kind", save_str("holstering-primary")),
            ("next", reference("q2:base", "q2:weapon_blaster")),
        ]);
        assert!(matches!(
            read_weapon_slot_state(SaveReader::new(&holstering)).expect("holster"),
            WeaponSlotRestoreState::HolsteringPrimary { .. }
        ));
        let json = obj(vec![
            ("kind", save_str("holstering-equipment")),
            ("next", reference("q1:threewave", "q1:ctf/weapon/grapple")),
        ]);
        assert!(matches!(
            read_weapon_slot_state(SaveReader::new(&json)).expect("holster"),
            WeaponSlotRestoreState::HolsteringEquipment { .. }
        ));
    }

    #[test]
    fn reads_slot_entries() {
        let json = arr(vec![obj(vec![
            ("actor", write_saved_actor(SavedActorId { slot: 3, generation: 1 })),
            (
                "state",
                obj(vec![("kind", save_str("active")), ("provider", save_str("q2:base"))]),
            ),
        ])]);
        assert_eq!(
            read_weapon_slots(SaveReader::new(&json)).expect("slots"),
            vec![WeaponSlotEntry {
                actor: SavedActorId { slot: 3, generation: 1 },
                state: WeaponSlotRestoreState::State(WeaponSlotState::Active {
                    provider: ProviderId::new("q2", "base"),
                }),
            }]
        );
    }

    #[test]
    fn reads_grapple_weapon_state() {
        let json = obj(vec![
            ("handoff", save_str("holstering")),
            (
                "animation",
                obj(vec![
                    ("phase", save_str("firing")),
                    ("frame", int(4)),
                    ("latchedAttack", boolean(true)),
                    ("sourceFiring", boolean(false)),
                    ("thinkTime", num(1.5)),
                    ("fireFinished", num(2.5)),
                    ("fireBuffered", boolean(true)),
                    ("lastFiringTime", num(0.5)),
                ]),
            ),
        ]);
        assert_eq!(
            read_grapple_weapon_state(SaveReader::new(&json)).expect("grapple"),
            GrappleWeaponState {
                handoff: GrappleHandoff::Holstering,
                animation: Q2GenericFrameState {
                    phase: Q2WeaponPhase::Firing,
                    frame: 4,
                    latched_attack: true,
                    source_firing: false,
                    think_time: 1.5,
                    fire_finished: 2.5,
                    fire_buffered: true,
                    last_firing_time: 0.5,
                },
            }
        );
    }

    #[test]
    fn rejects_bad_slot_kind() {
        let json = obj(vec![("kind", save_str("lowered"))]);
        assert!(read_weapon_slot_state(SaveReader::new(&json)).is_err());
    }
}
