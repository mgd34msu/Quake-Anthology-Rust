//! Native primary-weapon host binding for guest worlds.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/native-primary-weapons.ts`.
//!
//! The donor reads live guest internals (host memory, runner, image base,
//! edict tables) off the classic or rerelease world. Those internals belong
//! to the guest partition, so [`NativePrimaryWeaponWorld`] seams exactly the
//! members the donor builder touches, and [`bind_native_primary_weapons`]
//! takes the `NativePrimaryWeapons` constructor as a seam: the Rust service
//! only binds synthetic hosts today.

use std::rc::Rc;

use qa_compat::q2::native_primary_weapons::NativePrimaryWeaponProfile;
use qa_core::identity::{ActorId, ProviderId};
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallSignature, GuestCallValue, RawEntityView};
use qa_world::registry::ActorRegistry;

/// Guest entry invocation.
pub type PrimaryWeaponInvoke<'w> =
    Rc<dyn Fn(GuestAddress, &GuestCallSignature, &[GuestCallValue]) -> GuestCallResult + 'w>;
/// Guest entry lookup.
pub type PrimaryWeaponEntry<'w> = Rc<dyn Fn(&str) -> GuestAddress + 'w>;
/// Entity record lookup.
pub type PrimaryWeaponRecord<'w> = Rc<dyn Fn(GuestAddress) -> RawEntityView + 'w>;
/// Current actor behind a record.
pub type PrimaryWeaponActor<'w> = Rc<dyn Fn(&RawEntityView) -> Option<ActorId> + 'w>;
/// Record behind a live actor.
pub type PrimaryWeaponRecordFor<'w> = Rc<dyn Fn(&ActorId) -> Option<RawEntityView> + 'w>;
/// Weapon selection probe.
pub type PrimaryWeaponSelected = Box<dyn FnMut(&ActorId) -> bool>;
/// Dispatcher completion notice.
pub type PrimaryWeaponCompleted = Box<dyn FnMut(&ActorId, bool)>;
/// Spawn acceptance notice.
pub type PrimaryWeaponSpawned = Box<dyn FnMut(&ActorId)>;

/// Mirror of `NativePrimaryWeaponHost` from donor
/// `src/compat/q2/native-primary-weapons.ts` (canonical home:
/// `qa_compat::q2::native_primary_weapons`); unify post-merge.
///
/// Memory and runner stay generic: the mapped guest types land with the
/// guest partition, and the bound host only forwards them.
pub struct NativePrimaryWeaponHost<'w, M: ?Sized, R: ?Sized> {
    /// Guest memory.
    pub memory: &'w M,
    /// Guest call runner.
    pub runner: &'w R,
    /// Loaded image base.
    pub image: GuestAddress,
    /// Invoke a guest entry.
    pub invoke: PrimaryWeaponInvoke<'w>,
    /// Resolve an entry address by name.
    pub entry: PrimaryWeaponEntry<'w>,
    /// Read the entity record behind an address.
    pub record: PrimaryWeaponRecord<'w>,
    /// Current actor behind a record, if bound.
    pub actor: PrimaryWeaponActor<'w>,
    /// Record behind a live actor of this module, if any.
    pub record_for: PrimaryWeaponRecordFor<'w>,
}

/// Mirror of `NativePrimaryWeaponHooks` from donor
/// `src/compat/q2/native-primary-weapons.ts` (canonical home:
/// `qa_compat::q2::native_primary_weapons`); unify post-merge.
pub struct NativePrimaryWeaponHooks {
    /// Whether the actor's weapon is selected.
    pub selected: PrimaryWeaponSelected,
    /// Dispatcher completion notice.
    pub completed: PrimaryWeaponCompleted,
    /// Spawn acceptance notice.
    pub spawned: PrimaryWeaponSpawned,
}

/// Guest-world access for primary-weapon binding (seam).
///
/// Donor `nativePrimaryWeaponHost` reads these members off
/// `ClassicGuestWorld | RereleaseGuestWorld` (donor
/// `src/app/bootstrap/simulation/native-primary-weapons.ts`): host memory,
/// runner, image base, invoke/entry dispatch, record lookup by pointer and
/// by slot, the current actor behind a record, and the module identity used
/// to scope `recordFor`. Both donor branches collapse into this one trait;
/// the guest partition implements it per edition.
pub trait NativePrimaryWeaponWorld {
    /// Guest memory type.
    type Memory: ?Sized;
    /// Guest call runner type.
    type Runner: ?Sized;
    /// Guest memory.
    fn memory(&self) -> &Self::Memory;
    /// Guest call runner.
    fn runner(&self) -> &Self::Runner;
    /// Loaded image base.
    fn image_base(&self) -> GuestAddress;
    /// Invoke a guest entry.
    fn invoke(
        &self,
        address: GuestAddress,
        signature: &GuestCallSignature,
        values: &[GuestCallValue],
    ) -> GuestCallResult;
    /// Resolve an entry address by name.
    fn entry(&self, name: &str) -> GuestAddress;
    /// Read the entity record behind an address.
    fn record_from_pointer(&self, address: GuestAddress) -> RawEntityView;
    /// Current actor behind a record, if bound.
    fn actor_of_record(&self, record: &RawEntityView) -> Option<ActorId>;
    /// Record at a source slot.
    fn record_at_slot(&self, slot: u32) -> RawEntityView;
    /// Owning module identity scoping `recordFor`.
    fn module_provider(&self) -> ProviderId;
}

/// Build the primary-weapon host over a guest world.
pub fn native_primary_weapon_host<'w, W: NativePrimaryWeaponWorld>(
    world: &'w W,
    actors: &'w ActorRegistry,
) -> NativePrimaryWeaponHost<'w, W::Memory, W::Runner> {
    NativePrimaryWeaponHost {
        memory: world.memory(),
        runner: world.runner(),
        image: world.image_base(),
        invoke: Rc::new(move |address, signature, values| world.invoke(address, signature, values)),
        entry: Rc::new(move |name| world.entry(name)),
        record: Rc::new(move |address| world.record_from_pointer(address)),
        actor: Rc::new(move |record| world.actor_of_record(record)),
        record_for: Rc::new(move |actor| {
            actors.source_of(actor).and_then(|(provider, slot)| {
                if provider == world.module_provider() {
                    Some(world.record_at_slot(slot))
                } else {
                    None
                }
            })
        }),
    }
}

/// Bind native primary weapons over a guest world.
///
/// The `bind` constructor is a seam for donor
/// `src/compat/q2/native-primary-weapons.ts` (`new NativePrimaryWeapons`):
/// the Rust service only binds synthetic hosts, so the live-host
/// constructor is injected and reported for post-merge unification.
pub fn bind_native_primary_weapons<'w, W: NativePrimaryWeaponWorld, Bound>(
    world: &'w W,
    actors: &'w ActorRegistry,
    profile: NativePrimaryWeaponProfile,
    hooks: NativePrimaryWeaponHooks,
    bind: impl FnOnce(
        NativePrimaryWeaponHost<'w, W::Memory, W::Runner>,
        NativePrimaryWeaponProfile,
        NativePrimaryWeaponHooks,
    ) -> Bound,
) -> Bound {
    bind(native_primary_weapon_host(world, actors), profile, hooks)
}

#[cfg(test)]
mod tests {
    use qa_compat::q2::native_primary_profiles::read_native_primary_weapons;
    use qa_compat::q2::native_primary_reader::{JsonValue, Reader};
    use qa_core::identity::IdentityOwner;
    use qa_guest::core::contracts::{ContentDigest, GuestLayout, ModuleIdentity, NativeAbi, NativeCallAbi};

    use super::*;

    struct StubWorld {
        provider: ProviderId,
        image: GuestAddress,
        memory: u32,
        runner: u32,
    }

    fn stub_view(slot: u32) -> RawEntityView {
        RawEntityView {
            module: ModuleIdentity {
                id: ProviderId::new("q2", "game"),
                digest: ContentDigest::new("sha256", "digest"),
                artifact_path: "game.dll".to_string(),
                revision: "1".to_string(),
            },
            slot,
            address: GuestAddress {
                space: 7,
                offset: 0x1000,
            },
            stride_bytes: 64,
            public_layout: GuestLayout {
                id: "entity".to_string(),
                byte_length: 64,
                alignment: 4,
                pointer_bytes: 4,
                fields: Vec::new(),
            },
            bytes: vec![0; 64],
        }
    }

    impl NativePrimaryWeaponWorld for StubWorld {
        type Memory = u32;
        type Runner = u32;

        fn memory(&self) -> &u32 {
            &self.memory
        }

        fn runner(&self) -> &u32 {
            &self.runner
        }

        fn image_base(&self) -> GuestAddress {
            self.image
        }

        fn invoke(
            &self,
            _address: GuestAddress,
            _signature: &GuestCallSignature,
            values: &[GuestCallValue],
        ) -> GuestCallResult {
            GuestCallResult::Value(values.first().cloned().unwrap_or(GuestCallValue::Int32(0)))
        }

        fn entry(&self, _name: &str) -> GuestAddress {
            GuestAddress {
                space: 7,
                offset: 0x2000,
            }
        }

        fn record_from_pointer(&self, address: GuestAddress) -> RawEntityView {
            stub_view(address.offset as u32)
        }

        fn actor_of_record(&self, record: &RawEntityView) -> Option<ActorId> {
            (record.slot != 0).then(|| IdentityOwner::create("record-test").unwrap().actor(record.slot, 1))
        }

        fn record_at_slot(&self, slot: u32) -> RawEntityView {
            stub_view(slot)
        }

        fn module_provider(&self) -> ProviderId {
            self.provider.clone()
        }
    }

    fn world(provider: ProviderId) -> StubWorld {
        StubWorld {
            provider,
            image: GuestAddress { space: 7, offset: 0 },
            memory: 42,
            runner: 7,
        }
    }

    fn registry() -> ActorRegistry {
        ActorRegistry::new(IdentityOwner::create("primary-test").unwrap(), 8).unwrap()
    }

    fn obj(fields: &[(&str, JsonValue)]) -> JsonValue {
        JsonValue::Object(
            fields
                .iter()
                .map(|(key, value)| ((*key).to_string(), value.clone()))
                .collect(),
        )
    }

    fn field() -> JsonValue {
        obj(&[
            ("record", JsonValue::Str("entity".to_string())),
            ("offset", JsonValue::Int(4)),
            ("encoding", JsonValue::Str("int32".to_string())),
        ])
    }

    /// A profile through the sanctioned JSON reader: delay/damage variants
    /// keep private fields, so cross-crate tests cannot use literals.
    fn profile() -> NativePrimaryWeaponProfile {
        let json = obj(&[
            (
                "dispatcher",
                obj(&[
                    (
                        "entry",
                        obj(&[
                            ("kind", JsonValue::Str("rva".to_string())),
                            ("rva", JsonValue::Int(0x100)),
                        ]),
                    ),
                    ("argument", JsonValue::Int(0)),
                    ("arguments", JsonValue::Int(1)),
                    ("record", JsonValue::Str("entity".to_string())),
                ]),
            ),
            ("decisions", JsonValue::Array(Vec::new())),
            (
                "spawn",
                obj(&[
                    ("entry", JsonValue::Int(0x200)),
                    ("accepted", JsonValue::Array(Vec::new())),
                ]),
            ),
            ("active", JsonValue::Array(Vec::new())),
            ("committedInput", JsonValue::Array(Vec::new())),
            ("continuations", JsonValue::Array(Vec::new())),
            (
                "time",
                obj(&[
                    ("address", JsonValue::Int(0x300)),
                    ("encoding", JsonValue::Str("float32".to_string())),
                    ("milliseconds", JsonValue::Float(1000.0)),
                ]),
            ),
            (
                "entity",
                obj(&[
                    ("client", JsonValue::Int(0)),
                    ("waterLevel", field()),
                    ("viewHeight", field()),
                    ("maxHealth", field()),
                ]),
            ),
            (
                "client",
                obj(&[
                    ("byteLength", JsonValue::Int(128)),
                    ("viewAngles", JsonValue::Int(8)),
                    ("buttons", field()),
                    ("latchedButtons", field()),
                ]),
            ),
            (
                "attackAnimation",
                obj(&[("entry", JsonValue::Int(0x400)), ("skip", JsonValue::Array(Vec::new()))]),
            ),
            (
                "animation",
                obj(&[
                    ("frame", field()),
                    ("end", field()),
                    ("priority", field()),
                    ("duck", field()),
                    ("run", field()),
                ]),
            ),
            ("equipmentContexts", JsonValue::Array(Vec::new())),
            (
                "delay",
                obj(&[
                    ("flag", field()),
                    (
                        "region",
                        obj(&[("entry", JsonValue::Int(1)), ("join", JsonValue::Int(2))]),
                    ),
                    (
                        "evaluate",
                        obj(&[
                            ("kind", JsonValue::Str("source-flag".to_string())),
                            ("factors", JsonValue::Array(vec![JsonValue::Float(1.0)])),
                        ]),
                    ),
                ]),
            ),
            (
                "damage",
                obj(&[
                    ("kind", JsonValue::Str("source-result".to_string())),
                    ("entry", JsonValue::Int(0x500)),
                    ("result", JsonValue::Str("int32".to_string())),
                ]),
            ),
        ]);
        read_native_primary_weapons(&Reader::root(&json), "digest".to_string(), NativeAbi::WindowsX86_64)
    }

    #[test]
    fn host_forwards_world_dispatch() {
        let mut actors = registry();
        let provider = ProviderId::new("q2", "game");
        let stub = world(provider.clone());
        let owned = actors.allocate_at_source(provider.clone(), 5, "player").unwrap();
        let host = native_primary_weapon_host(&stub, &actors);
        assert_eq!(host.memory, &42);
        assert_eq!(host.runner, &7);
        assert_eq!(host.image, GuestAddress { space: 7, offset: 0 });
        assert_eq!(
            (host.entry)("Spawn"),
            GuestAddress {
                space: 7,
                offset: 0x2000
            }
        );
        let address = GuestAddress { space: 7, offset: 9 };
        assert_eq!((host.record)(address).slot, 9);
        assert!((host.actor)(&stub_view(0)).is_none());
        assert!((host.actor)(&stub_view(3)).is_some());
        let signature = GuestCallSignature {
            abi: NativeCallAbi::Cdecl,
            parameters: Vec::new(),
            result: None,
            variadic: false,
        };
        assert_eq!(
            (host.invoke)(address, &signature, &[GuestCallValue::Int32(5)]),
            GuestCallResult::Value(GuestCallValue::Int32(5))
        );
        // The allocated actor resolves through the module provider.
        let record = (host.record_for)(owned.id()).unwrap();
        assert_eq!(record.slot, 5);
    }

    #[test]
    fn record_for_rejects_foreign_providers() {
        let mut actors = registry();
        let stub = world(ProviderId::new("q2", "game"));
        let foreign = actors
            .allocate_at_source(ProviderId::new("q1", "game"), 5, "player")
            .unwrap();
        let unbound = actors.allocate(ProviderId::new("q2", "game"), "item").unwrap();
        let host = native_primary_weapon_host(&stub, &actors);
        assert!((host.record_for)(foreign.id()).is_none());
        assert!((host.record_for)(unbound.id()).is_none());
        let retired = IdentityOwner::create("other").unwrap().actor(9, 1);
        assert!((host.record_for)(&retired).is_none());
    }

    #[test]
    fn bind_injects_constructor_with_host_profile_hooks() {
        let actors = registry();
        let stub = world(ProviderId::new("q2", "game"));
        let selected = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = selected.clone();
        let hooks = NativePrimaryWeaponHooks {
            selected: Box::new(move |_| {
                seen.set(true);
                true
            }),
            completed: Box::new(|_, _| {}),
            spawned: Box::new(|_| {}),
        };
        let actor = IdentityOwner::create("hook-test").unwrap().actor(1, 1);
        let bound = bind_native_primary_weapons(&stub, &actors, profile(), hooks, |host, profile, mut hooks| {
            assert_eq!(profile.digest, "digest");
            assert_eq!(host.image.offset, 0);
            assert!((hooks.selected)(&actor));
            "bound"
        });
        assert_eq!(bound, "bound");
        assert!(selected.get());
    }
}
