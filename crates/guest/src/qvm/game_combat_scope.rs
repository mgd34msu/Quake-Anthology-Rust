//! Suspended QVM damage frames following every source store.
//!
//! Provenance: `src/compat/qvm/game-combat-scope.ts`.
//!
//! Local mirrors: `src/world/gameplay/authority.ts` ([`SourceDamageObserver`],
//! [`SourceDamageResult`]) and the `DamageRequest` attack provenance
//! ([`QvmScopeDamageRequest`]); armor states reuse
//! [`qa_world::combat::ArmorState`].
//!
//! The donor throws cancellations across the interpreter boundary; this port
//! records cancellation on the frame and returns immediately, which is
//! equivalent inside the synchronous mirror. The donor `run` proceeds the
//! source call directly; the mirror accepts an optional `invoke` seam whose
//! default is exactly `call.proceed()`, so fixtures can stand in for source
//! execution (stores, nested reactions) without an interpreter.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_world::combat::{ArmorState, RegularArmor};

use super::game_combat::{QvmDamageCause, QvmReactionCall};
use super::game_data::{
    QvmCommittedWrite, QvmFunctionCall, QvmFunctionObservation, QvmGameData, QvmModule, QvmObserveFn, QvmWatchCallback,
    QvmWriteRange,
};
use crate::error::GuestError;

/// Damage request fields a scope reports (mirror of the attack provenance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmScopeDamageRequest {
    /// Damage cause.
    pub cause: QvmDamageCause,
    /// Namespaced movement provider.
    pub movement_provider: String,
}

/// Stored damage change published to the observer.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmStoredDamage {
    /// Health store.
    Health {
        /// Health before the store.
        before: i32,
        /// Health after the store.
        after: i32,
    },
    /// Armor store.
    Armor {
        /// Armor before the store.
        before: ArmorState,
        /// Armor after the store.
        after: ArmorState,
    },
    /// Source velocity store.
    SourceVelocity {
        /// Velocity before the store.
        before: Vec3,
        /// Velocity after the store.
        after: Vec3,
        /// Namespaced movement provider.
        movement_provider: String,
    },
}

/// Damage reaction kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmDamageReaction {
    /// No reaction.
    None,
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Death,
}

/// Damage outcome reported to the observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceDamageResult {
    /// Applied damage.
    pub applied_damage: i32,
    /// Reaction kind.
    pub reaction: QvmDamageReaction,
}

/// Damage observer (mirror of `SourceDamageObserver`).
///
/// Observers run synchronously inside source stores; they must not reenter
/// the same scope run through this handle (nested runs use their own
/// observer handles).
pub trait SourceDamageObserver {
    /// Report a stored change.
    fn stored(&mut self, stored: QvmStoredDamage);
    /// Report an imminent reaction.
    fn before_reaction(&mut self, result: &SourceDamageResult);
}

/// Shared damage observer handle captured by write closures.
pub type SharedDamageObserver = Rc<RefCell<dyn SourceDamageObserver>>;

/// Declared pain/death reaction entries and calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmDamageReactions {
    /// Entity offset holding the pain entry.
    pub pain: usize,
    /// Entity offset holding the death entry.
    pub die: usize,
    /// Pain reaction call.
    pub pain_call: QvmReactionCall,
    /// Death reaction call.
    pub die_call: QvmReactionCall,
}

/// Damage-scope options.
#[derive(Clone)]
pub struct QvmDamageScopeOptions {
    /// Guest module.
    pub module: QvmModule,
    /// Located game data.
    pub data: QvmGameData,
    /// Entity health offset.
    pub health: usize,
    /// Target argument position.
    pub target_argument: usize,
    /// Armor points stat.
    pub points_stat: i32,
    /// Armor tier stat, if any.
    pub tier_stat: Option<i32>,
    /// Absolute mode-word byte offsets.
    pub mode_words: Vec<usize>,
    /// Reaction declarations.
    pub reactions: QvmDamageReactions,
    /// Read current armor for a slot.
    pub armor: Rc<dyn Fn(usize) -> ArmorState>,
    /// Check actor liveness.
    pub live: Rc<dyn Fn(&ActorId) -> bool>,
}

/// One suspended damage frame (shared handle; clones alias one frame).
#[derive(Debug, Clone)]
pub struct QvmDamageFrame {
    inner: Rc<RefCell<QvmDamageFrameInner>>,
}

#[derive(Debug)]
struct QvmDamageFrameInner {
    actor: ActorId,
    pointer: i32,
    cancelled: bool,
    reacting: bool,
}

impl QvmDamageFrame {
    /// Frame actor.
    #[must_use]
    pub fn actor(&self) -> ActorId {
        self.inner.borrow().actor.clone()
    }

    /// Frame target pointer.
    #[must_use]
    pub fn pointer(&self) -> i32 {
        self.inner.borrow().pointer
    }

    /// Whether the frame was cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.borrow().cancelled
    }

    /// Whether the frame is inside a reaction.
    #[must_use]
    pub fn is_reacting(&self) -> bool {
        self.inner.borrow().reacting
    }
}

/// Invocation cancelled through a scope.
pub trait QvmCancellable {
    /// Cancel the invocation.
    fn cancel(&mut self);
}

impl QvmCancellable for QvmFunctionCall {
    fn cancel(&mut self) {
        self.cancel_function();
    }
}

impl QvmCancellable for QvmFunctionObservation {
    fn cancel(&mut self) {
        self.cancel_function();
    }
}

fn same_armor(a: &ArmorState, b: &ArmorState) -> bool {
    match (&a.regular, &b.regular) {
        (RegularArmor::None, RegularArmor::None) => true,
        (
            RegularArmor::Q3 {
                points: a_points,
                protection: a_protection,
            },
            RegularArmor::Q3 {
                points: b_points,
                protection: b_protection,
            },
        ) => a_points == b_points && a_protection == b_protection,
        _ => false,
    }
}

#[derive(Debug, Clone)]
enum StoredChange {
    Health { before: i32, after: i32 },
    Armor { before: ArmorState, after: ArmorState },
    Velocity { before: Vec3, after: Vec3 },
}

struct ReactionState {
    entry: i32,
    hook: Option<u64>,
}

/// Suspended hits follow every store; the innermost hit alone reports it.
#[derive(Clone)]
pub struct QvmDamageScopes {
    options: QvmDamageScopeOptions,
    frames: Rc<RefCell<Vec<QvmDamageFrame>>>,
}

impl std::fmt::Debug for QvmDamageScopes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QvmDamageScopes")
            .field("frames", &self.frames.borrow().len())
            .finish()
    }
}

impl QvmDamageScopes {
    /// Build scopes over module, data, and armor/liveness callbacks.
    #[must_use]
    pub fn new(options: QvmDamageScopeOptions) -> Self {
        Self {
            options,
            frames: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Innermost non-reacting frame for `pointer`, if any.
    #[must_use]
    pub fn current(&self, pointer: i32) -> Option<QvmDamageFrame> {
        let frames = self.frames.borrow();
        let frame = frames.last()?;
        if frame.pointer() == pointer && !frame.is_reacting() {
            Some(frame.clone())
        } else {
            None
        }
    }

    /// Cancel through `target`, recording the request on the frame.
    pub fn cancel(&self, frame: &QvmDamageFrame, target: &mut dyn QvmCancellable) {
        target.cancel();
        frame.inner.borrow_mut().cancelled = true;
    }

    /// Run a damage call, reporting stores and reactions to the observer.
    ///
    /// `invoke` stands in for source execution; `None` proceeds the call.
    pub fn run(
        &self,
        call: &mut QvmFunctionCall,
        actor: &ActorId,
        slot: usize,
        request: &QvmScopeDamageRequest,
        observer: &SharedDamageObserver,
        invoke: Option<&dyn Fn(&mut QvmFunctionCall) -> i32>,
    ) -> Result<SourceDamageResult, GuestError> {
        let options = &self.options;
        let entity = options.data.entity_bytes(slot)?;
        let frame = QvmDamageFrame {
            inner: Rc::new(RefCell::new(QvmDamageFrameInner {
                actor: actor.clone(),
                pointer: call.argument(options.target_argument)?,
                cancelled: false,
                reacting: false,
            })),
        };
        let entity_offset = entity.offset;
        let health_range = QvmWriteRange {
            byte_offset: entity_offset + options.health,
            byte_length: 4,
        };
        let client = if slot < options.data.num_clients() {
            Some(options.data.client_bytes(slot)?)
        } else {
            None
        };
        let client_offset = client.as_ref().map(|window| window.offset);
        let stat_range = |base: usize, stat: i32| -> Result<QvmWriteRange, GuestError> {
            let offset = i64::try_from(base)
                .map_err(|_| GuestError::invalid("QVM client stat is outside the allocation"))?
                + 184
                + i64::from(stat) * 4;
            let offset = usize::try_from(offset)
                .map_err(|_| GuestError::invalid("QVM client stat is outside the allocation"))?;
            Ok(QvmWriteRange {
                byte_offset: offset,
                byte_length: 4,
            })
        };
        let mut armor_ranges: Vec<QvmWriteRange> = Vec::new();
        if let Some(base) = client_offset {
            armor_ranges.push(stat_range(base, options.points_stat)?);
            if let Some(tier) = options.tier_stat {
                armor_ranges.push(stat_range(base, tier)?);
            }
            for mode in &options.mode_words {
                armor_ranges.push(QvmWriteRange {
                    byte_offset: *mode,
                    byte_length: 4,
                });
            }
        }
        let velocity_range = QvmWriteRange {
            byte_offset: client_offset.map_or(entity_offset + 36, |base| base + 32),
            byte_length: 12,
        };
        let memory = options.module.memory();
        let health_cell = Rc::new(RefCell::new(entity.get_i32(options.health)?));
        let armor_cell = Rc::new(RefCell::new((options.armor)(slot)));
        let velocity_cell = Rc::new(RefCell::new(memory.read_vec3(velocity_range.byte_offset)?));
        let result_cell = Rc::new(RefCell::new(SourceDamageResult {
            applied_damage: 0,
            reaction: QvmDamageReaction::None,
        }));

        let frames = Rc::clone(&self.frames);
        let is_current = {
            let frames = Rc::clone(&frames);
            let frame = frame.clone();
            let actor = actor.clone();
            let live = Rc::clone(&options.live);
            Rc::new(move || {
                let frames = frames.borrow();
                for entry in frames.iter().rev() {
                    if entry.actor() == actor {
                        return Rc::ptr_eq(&entry.inner, &frame.inner) && !frame.is_reacting() && live(&actor);
                    }
                }
                false
            })
        };

        frames.borrow_mut().push(frame.clone());
        let mut watches: Vec<u64> = Vec::new();
        let mut reactions: Vec<Rc<RefCell<ReactionState>>> = Vec::new();
        let outcome = (|| -> Result<SourceDamageResult, GuestError> {
            {
                let entity = entity.clone();
                let health = options.health;
                let health_cell = Rc::clone(&health_cell);
                let armor_cell = Rc::clone(&armor_cell);
                let velocity_cell = Rc::clone(&velocity_cell);
                let result_cell = Rc::clone(&result_cell);
                let armor_of = Rc::clone(&options.armor);
                let is_current = Rc::clone(&is_current);
                let observer = Rc::clone(observer);
                let movement_provider = request.movement_provider.clone();
                let ranges = armor_ranges.clone();
                let memory = memory.clone();
                let publish: QvmWatchCallback = Rc::new(move |event: &QvmCommittedWrite| {
                    let report = is_current();
                    let mut changes: Vec<(usize, StoredChange)> = Vec::new();
                    if event.touches(&health_range) {
                        if let Ok(next) = entity.get_i32(health) {
                            let before = *health_cell.borrow();
                            *health_cell.borrow_mut() = next;
                            if report && before != next {
                                changes.push((health_range.byte_offset, StoredChange::Health { before, after: next }));
                            }
                        }
                    }
                    if ranges.iter().any(|range| event.touches(range)) {
                        let before = armor_cell.borrow().clone();
                        let next = armor_of(slot);
                        *armor_cell.borrow_mut() = next.clone();
                        if report && !same_armor(&before, &next) {
                            let offset = ranges
                                .iter()
                                .find(|range| event.touches(range))
                                .map_or(0, |range| range.byte_offset);
                            changes.push((offset, StoredChange::Armor { before, after: next }));
                        }
                    }
                    if event.touches(&velocity_range) {
                        if let Ok(next) = memory.read_vec3(velocity_range.byte_offset) {
                            let before = velocity_cell.borrow().clone();
                            *velocity_cell.borrow_mut() = next.clone();
                            if report && before != next {
                                changes.push((
                                    velocity_range.byte_offset,
                                    StoredChange::Velocity { before, after: next },
                                ));
                            }
                        }
                    }
                    changes.sort_by_key(|(offset, _)| *offset);
                    for (_, change) in changes {
                        match change {
                            StoredChange::Health { before, after } => {
                                observer.borrow_mut().stored(QvmStoredDamage::Health { before, after });
                                result_cell.borrow_mut().applied_damage += before - after;
                            }
                            StoredChange::Armor { before, after } => {
                                observer.borrow_mut().stored(QvmStoredDamage::Armor { before, after });
                            }
                            StoredChange::Velocity { before, after } => {
                                observer.borrow_mut().stored(QvmStoredDamage::SourceVelocity {
                                    before,
                                    after,
                                    movement_provider: movement_provider.clone(),
                                });
                            }
                        }
                    }
                });
                let mut all = vec![health_range, velocity_range];
                all.extend(ranges);
                watches.push(memory.observe_writes(all, publish, None));
            }

            for reaction in [QvmDamageReaction::Pain, QvmDamageReaction::Death] {
                let (offset, roles) = match reaction {
                    QvmDamageReaction::Pain => (options.reactions.pain, options.reactions.pain_call.clone()),
                    _ => (options.reactions.die, options.reactions.die_call.clone()),
                };
                let state = Rc::new(RefCell::new(ReactionState { entry: 0, hook: None }));
                reactions.push(Rc::clone(&state));
                let refresh = {
                    let state = Rc::clone(&state);
                    let entity = entity.clone();
                    let frame = frame.clone();
                    let module = options.module.clone();
                    let is_current = Rc::clone(&is_current);
                    let live = Rc::clone(&options.live);
                    let result_cell = Rc::clone(&result_cell);
                    let observer = Rc::clone(observer);
                    let actor = actor.clone();
                    let scopes = self.clone();
                    Rc::new(move || -> Result<(), GuestError> {
                        let next = entity.get_i32(offset)?;
                        let mut state = state.borrow_mut();
                        if next == state.entry {
                            return Ok(());
                        }
                        if let Some(hook) = state.hook.take() {
                            module.remove_hook(hook);
                        }
                        state.entry = next;
                        if next == 0 {
                            return Ok(());
                        }
                        let entry = usize::try_from(next)
                            .map_err(|_| GuestError::invalid("QVM reaction entry is not a function index"))?;
                        let entity = entity.clone();
                        let frame = frame.clone();
                        let is_current = Rc::clone(&is_current);
                        let live = Rc::clone(&live);
                        let result_cell = Rc::clone(&result_cell);
                        let observer = Rc::clone(&observer);
                        let actor = actor.clone();
                        let scopes = scopes.clone();
                        let watch: QvmObserveFn = Rc::new(move |reaction_call| {
                            let target = reaction_call.argument(roles.target).unwrap_or(-1);
                            let amount = reaction_call.argument(roles.amount).unwrap_or(0);
                            if entity.get_i32(offset).unwrap_or(-1) != next
                                || !is_current()
                                || target != frame.pointer()
                            {
                                return;
                            }
                            frame.inner.borrow_mut().reacting = true;
                            let outcome = SourceDamageResult {
                                applied_damage: amount,
                                reaction,
                            };
                            *result_cell.borrow_mut() = outcome;
                            observer.borrow_mut().before_reaction(&outcome);
                            if !live(&actor) {
                                scopes.cancel(&frame, reaction_call);
                            }
                        });
                        state.hook = Some(module.observe_function(entry, watch));
                        Ok(())
                    })
                };
                let trigger = Rc::clone(&refresh);
                watches.push(memory.observe_writes(
                    vec![QvmWriteRange {
                        byte_offset: entity_offset + offset,
                        byte_length: 4,
                    }],
                    Rc::new(move |_| {
                        let _ = trigger();
                    }),
                    None,
                ));
                refresh()?;
            }

            match invoke {
                Some(invoke) => {
                    invoke(call);
                }
                None => {
                    call.proceed();
                }
            }
            Ok(*result_cell.borrow())
        })();

        for watch in watches {
            memory.remove_observer(watch);
        }
        for state in &reactions {
            if let Some(hook) = state.borrow_mut().hook.take() {
                options.module.remove_hook(hook);
            }
        }
        frames.borrow_mut().pop();
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::super::game_data::{AbiProfile, QvmArtifact, QvmImage, QvmRole, QvmSharedMemory};
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_world::combat::PoweredProtection;

    struct FixtureObserver {
        stored: Vec<QvmStoredDamage>,
        reactions: Vec<SourceDamageResult>,
    }

    impl SourceDamageObserver for FixtureObserver {
        fn stored(&mut self, stored: QvmStoredDamage) {
            self.stored.push(stored);
        }

        fn before_reaction(&mut self, result: &SourceDamageResult) {
            self.reactions.push(*result);
        }
    }

    fn armor_none() -> ArmorState {
        ArmorState {
            regular: RegularArmor::None,
            powered: PoweredProtection::None,
        }
    }

    fn fixture() -> (QvmDamageScopes, QvmSharedMemory, ActorId) {
        let artifact = QvmArtifact {
            module: super::super::game_data::ModuleIdentity {
                id: "q3:qagame".to_string(),
                artifact_path: "qagame.qvm".to_string(),
                digest: "d".to_string(),
                revision: "r".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: None,
            image: QvmImage::default(),
        };
        let module = QvmModule::new(artifact, None, None).unwrap();
        let memory = module.memory();
        let data = QvmGameData::new(memory.clone(), AbiProfile::Modern);
        data.locate(64, 2, 560, 4096, 480).unwrap();
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(0, 1);
        let live_actor = actor.clone();
        let scopes = QvmDamageScopes::new(QvmDamageScopeOptions {
            module,
            data,
            health: 520,
            target_argument: 0,
            points_stat: 1,
            tier_stat: None,
            mode_words: Vec::new(),
            reactions: QvmDamageReactions {
                pain: 532,
                die: 536,
                pain_call: QvmReactionCall {
                    arguments: 2,
                    target: 0,
                    amount: 1,
                },
                die_call: QvmReactionCall {
                    arguments: 2,
                    target: 0,
                    amount: 1,
                },
            },
            armor: Rc::new(move |_| armor_none()),
            live: Rc::new(move |candidate| candidate == &live_actor),
        });
        (scopes, memory, actor)
    }

    fn request() -> QvmScopeDamageRequest {
        QvmScopeDamageRequest {
            cause: QvmDamageCause::Environment,
            movement_provider: "q3:pmove".to_string(),
        }
    }

    #[test]
    fn health_and_velocity_stores_report_in_offset_order() {
        let (scopes, memory, actor) = fixture();
        memory.write_i32(64 + 520, 100).unwrap();
        memory.write_vec3(4096 + 32, &vec3(0.0, 0.0, 0.0)).unwrap();
        let observer: SharedDamageObserver = Rc::new(RefCell::new(FixtureObserver {
            stored: Vec::new(),
            reactions: Vec::new(),
        }));
        let mut call = QvmFunctionCall::entered(3, vec![64], memory.clone());
        let writer = memory.clone();
        let outcome = scopes
            .run(
                &mut call,
                &actor,
                0,
                &request(),
                &observer,
                Some(&|call| {
                    writer.write_vec3(4096 + 32, &vec3(0.0, 0.0, 300.0)).unwrap();
                    writer.write_i32(64 + 520, 75).unwrap();
                    call.proceed()
                }),
            )
            .unwrap();
        assert_eq!(outcome.applied_damage, 25);
        let stored = &observer.borrow().stored;
        assert_eq!(stored.len(), 2);
        assert!(matches!(stored[0], QvmStoredDamage::SourceVelocity { .. }));
        assert!(matches!(stored[1], QvmStoredDamage::Health { before: 100, after: 75 }));
        assert!(scopes.current(64).is_none());
    }

    #[test]
    fn current_selects_innermost_matching_frame() {
        let (scopes, memory, actor) = fixture();
        assert!(scopes.current(64).is_none());
        let observer: SharedDamageObserver = Rc::new(RefCell::new(FixtureObserver {
            stored: Vec::new(),
            reactions: Vec::new(),
        }));
        let mut call = QvmFunctionCall::entered(3, vec![64], memory.clone());
        let probe = scopes.clone();
        let expected = actor.clone();
        let seen = Rc::new(RefCell::new(false));
        let seen_hook = Rc::clone(&seen);
        scopes
            .run(
                &mut call,
                &actor,
                0,
                &request(),
                &observer,
                Some(&|call| {
                    let found = probe.current(64).is_some()
                        && probe.current(65).is_none()
                        && probe.current(64).unwrap().actor() == expected;
                    *seen_hook.borrow_mut() = found;
                    call.proceed()
                }),
            )
            .unwrap();
        assert!(*seen.borrow());
    }

    #[test]
    fn pain_reaction_reports_and_cancels_dead_actors() {
        let (scopes, memory, actor) = fixture();
        memory.write_i32(64 + 532, 11).unwrap();
        let observer: SharedDamageObserver = Rc::new(RefCell::new(FixtureObserver {
            stored: Vec::new(),
            reactions: Vec::new(),
        }));
        let mut call = QvmFunctionCall::entered(3, vec![64], memory.clone());
        let module = scopes.options.module.clone();
        let frame_seen = Rc::new(RefCell::new(None));
        let frame_hook = Rc::clone(&frame_seen);
        let scopes_hook = scopes.clone();
        let outcome = scopes
            .run(
                &mut call,
                &actor,
                0,
                &request(),
                &observer,
                Some(&|call| {
                    *frame_hook.borrow_mut() = scopes_hook.current(64);
                    module.call(&[64, 30], 11).unwrap();
                    call.proceed()
                }),
            )
            .unwrap();
        assert_eq!(outcome.reaction, QvmDamageReaction::Pain);
        assert_eq!(outcome.applied_damage, 30);
        assert_eq!(observer.borrow().reactions.len(), 1);
        assert!(!frame_hook.borrow().as_ref().unwrap().is_cancelled());
    }

    #[test]
    fn same_armor_ignores_powered_state() {
        let plain = armor_none();
        let suited = ArmorState {
            regular: RegularArmor::None,
            powered: PoweredProtection::Screen { cells: 5 },
        };
        assert!(same_armor(&plain, &suited));
        let plated = ArmorState {
            regular: RegularArmor::Q3 {
                points: 50.0,
                protection: 0.6,
            },
            powered: PoweredProtection::None,
        };
        let worn = ArmorState {
            regular: RegularArmor::Q3 {
                points: 40.0,
                protection: 0.6,
            },
            powered: PoweredProtection::None,
        };
        assert!(!same_armor(&plain, &plated));
        assert!(!same_armor(&plated, &worn));
        assert!(same_armor(&plated, &plated));
    }
}
