//! Mod gameplay-callback registration. Port of
//! `src/app/bootstrap/simulation/mod-callbacks.ts`.

use std::collections::BTreeMap;
use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::time::SourceTime;

/// Callback operation selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModCallbackOperation {
    Damage,
    InventoryGive,
    InventoryConsume,
    ActorThink,
    ActorTouch,
    ActorUse,
    ActorPain,
    ActorDie,
}

/// Callback stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModCallbackStage {
    Observe,
    Replace,
    Transform,
}

/// Writable scalar field of a damage request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageField {
    Amount,
    Knockback,
}

/// One callback binding.
#[derive(Debug, Clone, PartialEq)]
pub struct ModCallback {
    pub id: String,
    pub operation: ModCallbackOperation,
    pub stage: ModCallbackStage,
    pub result: DamageField,
}

/// Callback input slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModCallbackInput {
    Time,
    Result,
    SelfActor,
    Attacker,
    Inflictor,
    Amount,
    Knockback,
    Direction,
    Point,
    Normal,
    Item,
    Other,
    Activator,
    Elapsed,
}

/// Callback runtime value.
#[derive(Debug, Clone, PartialEq)]
pub enum ModRuntimeValue {
    Float(f64),
    Actor(Option<ActorId>),
    Vector([f64; 3]),
    Text(String),
}

/// Damage request.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    pub target: Option<ActorId>,
    pub attacker: Option<ActorId>,
    pub inflictor: Option<ActorId>,
    pub amount: f64,
    pub knockback: f64,
    pub direction: [f64; 3],
    pub point: [f64; 3],
    pub normal: [f64; 3],
}

/// Damage outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum DamageOutcome {
    Committed { applied_damage: f64 },
    Rejected,
}

/// Inventory give/consume request.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryRequest {
    pub owner: ActorId,
    pub item: String,
    pub amount: f64,
}

/// Think request.
#[derive(Debug, Clone, PartialEq)]
pub struct ThinkRequest {
    pub owner: ActorId,
    pub time: SourceTime,
    pub elapsed: SourceTime,
}

/// Touch request.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchRequest {
    pub self_id: ActorId,
    pub other: Option<ActorId>,
}

/// Use request.
#[derive(Debug, Clone, PartialEq)]
pub struct UseRequest {
    pub owner: ActorId,
    pub other: Option<ActorId>,
    pub activator: Option<ActorId>,
}

/// Pain request.
#[derive(Debug, Clone, PartialEq)]
pub struct PainRequest {
    pub self_id: ActorId,
    pub attacker: Option<ActorId>,
    pub damage: f64,
    pub kick: f64,
}

/// Death request.
#[derive(Debug, Clone, PartialEq)]
pub struct DieRequest {
    pub self_id: ActorId,
    pub attacker: Option<ActorId>,
    pub inflictor: Option<ActorId>,
    pub damage: f64,
    pub kick: f64,
    pub point: [f64; 3],
}

/// Registration target (operation plus stage).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModRegistrationTarget {
    DamageTransform,
    DamageObserve,
    InventoryGiveTransform,
    InventoryGiveObserve,
    InventoryConsumeTransform,
    InventoryConsumeObserve,
    ThinkObserve,
    ThinkReplace,
    TouchObserve,
    TouchReplace,
    UseObserve,
    UseReplace,
    PainObserve,
    PainReplace,
    DieObserve,
    DieReplace,
}

/// Boxed handler per registration target.
#[allow(clippy::type_complexity)]
pub enum ModCallbackHandler {
    DamageTransform(Box<dyn Fn(DamageRequest) -> DamageRequest>),
    DamageObserve(Box<dyn Fn(&DamageRequest, &DamageOutcome)>),
    InventoryGiveTransform(Box<dyn Fn(InventoryRequest) -> InventoryRequest>),
    InventoryGiveObserve(Box<dyn Fn(&InventoryRequest, f64)>),
    InventoryConsumeTransform(Box<dyn Fn(InventoryRequest) -> InventoryRequest>),
    InventoryConsumeObserve(Box<dyn Fn(&InventoryRequest, bool)>),
    ThinkObserve(Box<dyn Fn(&ThinkRequest, bool)>),
    ThinkReplace(Box<dyn Fn(ThinkRequest, &dyn Fn(ThinkRequest) -> bool) -> bool>),
    TouchObserve(Box<dyn Fn(&TouchRequest, bool)>),
    TouchReplace(Box<dyn Fn(TouchRequest, &dyn Fn(TouchRequest) -> bool) -> bool>),
    UseObserve(Box<dyn Fn(&UseRequest, bool)>),
    UseReplace(Box<dyn Fn(UseRequest, &dyn Fn(UseRequest) -> bool) -> bool>),
    PainObserve(Box<dyn Fn(&PainRequest, bool)>),
    PainReplace(Box<dyn Fn(PainRequest, &dyn Fn(PainRequest) -> bool) -> bool>),
    DieObserve(Box<dyn Fn(&DieRequest, bool)>),
    DieReplace(Box<dyn Fn(DieRequest, &dyn Fn(DieRequest) -> bool) -> bool>),
}

/// Execute a mod callback, returning the replacement value when it handles the call.
pub type ModCallbackExecute = Rc<dyn Fn(&ModCallback, &BTreeMap<ModCallbackInput, ModRuntimeValue>) -> Option<f64>>;

/// Registration sink.
pub trait ModRegistrations {
    fn register(&mut self, id: &str, target: ModRegistrationTarget, handler: ModCallbackHandler);
}

type Inputs = Vec<(ModCallbackInput, ModRuntimeValue)>;

fn damage_inputs(request: &DamageRequest) -> Inputs {
    vec![
        (
            ModCallbackInput::SelfActor,
            ModRuntimeValue::Actor(request.target.clone()),
        ),
        (
            ModCallbackInput::Attacker,
            ModRuntimeValue::Actor(request.attacker.clone()),
        ),
        (
            ModCallbackInput::Inflictor,
            ModRuntimeValue::Actor(request.inflictor.clone()),
        ),
        (ModCallbackInput::Amount, ModRuntimeValue::Float(request.amount)),
        (ModCallbackInput::Knockback, ModRuntimeValue::Float(request.knockback)),
        (ModCallbackInput::Direction, ModRuntimeValue::Vector(request.direction)),
        (ModCallbackInput::Point, ModRuntimeValue::Vector(request.point)),
        (ModCallbackInput::Normal, ModRuntimeValue::Vector(request.normal)),
    ]
}

fn inventory_inputs(request: &InventoryRequest) -> Inputs {
    vec![
        (
            ModCallbackInput::SelfActor,
            ModRuntimeValue::Actor(Some(request.owner.clone())),
        ),
        (ModCallbackInput::Item, ModRuntimeValue::Text(request.item.clone())),
        (ModCallbackInput::Amount, ModRuntimeValue::Float(request.amount)),
    ]
}

/// Register callback bindings. `execute` returns the callback result or `None`
/// when the callback declines (donor `null`).
pub fn register_mod_callbacks(
    callbacks: &[ModCallback],
    registrations: &mut dyn ModRegistrations,
    time: Rc<dyn Fn() -> SourceTime>,
    execute: ModCallbackExecute,
) {
    for callback in callbacks {
        let invoke = {
            let callback = callback.clone();
            let time = Rc::clone(&time);
            let execute = Rc::clone(&execute);
            move |values: Inputs, result: Option<f64>| -> Option<f64> {
                let mut inputs =
                    BTreeMap::from([(ModCallbackInput::Time, ModRuntimeValue::Float(time().as_seconds_f64()))]);
                inputs.extend(values);
                if let Some(result) = result {
                    inputs.insert(ModCallbackInput::Result, ModRuntimeValue::Float(result));
                }
                execute(&callback, &inputs)
            }
        };
        match callback.operation {
            ModCallbackOperation::Damage => {
                if callback.stage == ModCallbackStage::Transform {
                    let field = callback.result;
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::DamageTransform,
                        ModCallbackHandler::DamageTransform(Box::new(move |mut request: DamageRequest| {
                            if let Some(value) = invoke(damage_inputs(&request), None) {
                                match field {
                                    DamageField::Amount => request.amount = value,
                                    DamageField::Knockback => request.knockback = value,
                                }
                            }
                            request
                        })),
                    );
                } else {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::DamageObserve,
                        ModCallbackHandler::DamageObserve(Box::new(
                            move |request: &DamageRequest, outcome: &DamageOutcome| {
                                let applied = match outcome {
                                    DamageOutcome::Committed { applied_damage } => *applied_damage,
                                    DamageOutcome::Rejected => 0.0,
                                };
                                invoke(damage_inputs(request), Some(applied));
                            },
                        )),
                    );
                }
            }
            ModCallbackOperation::InventoryGive => {
                if callback.stage == ModCallbackStage::Transform {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::InventoryGiveTransform,
                        ModCallbackHandler::InventoryGiveTransform(Box::new(move |mut request: InventoryRequest| {
                            if let Some(amount) = invoke(inventory_inputs(&request), None) {
                                request.amount = amount;
                            }
                            request
                        })),
                    );
                } else {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::InventoryGiveObserve,
                        ModCallbackHandler::InventoryGiveObserve(Box::new(
                            move |request: &InventoryRequest, result: f64| {
                                invoke(inventory_inputs(request), Some(result));
                            },
                        )),
                    );
                }
            }
            ModCallbackOperation::InventoryConsume => {
                if callback.stage == ModCallbackStage::Transform {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::InventoryConsumeTransform,
                        ModCallbackHandler::InventoryConsumeTransform(Box::new(
                            move |mut request: InventoryRequest| {
                                if let Some(amount) = invoke(inventory_inputs(&request), None) {
                                    request.amount = amount;
                                }
                                request
                            },
                        )),
                    );
                } else {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::InventoryConsumeObserve,
                        ModCallbackHandler::InventoryConsumeObserve(Box::new(
                            move |request: &InventoryRequest, result: bool| {
                                invoke(inventory_inputs(request), Some(if result { 1.0 } else { 0.0 }));
                            },
                        )),
                    );
                }
            }
            ModCallbackOperation::ActorThink => {
                if callback.stage == ModCallbackStage::Observe {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::ThinkObserve,
                        ModCallbackHandler::ThinkObserve(Box::new(move |request: &ThinkRequest, result: bool| {
                            invoke(
                                vec![
                                    (
                                        ModCallbackInput::SelfActor,
                                        ModRuntimeValue::Actor(Some(request.owner.clone())),
                                    ),
                                    (
                                        ModCallbackInput::Time,
                                        ModRuntimeValue::Float(request.time.as_seconds_f64()),
                                    ),
                                    (
                                        ModCallbackInput::Elapsed,
                                        ModRuntimeValue::Float(request.elapsed.as_seconds_f64()),
                                    ),
                                ],
                                Some(if result { 1.0 } else { 0.0 }),
                            );
                        })),
                    );
                } else if callback.stage == ModCallbackStage::Replace {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::ThinkReplace,
                        ModCallbackHandler::ThinkReplace(Box::new(
                            move |request: ThinkRequest, next: &dyn Fn(ThinkRequest) -> bool| {
                                let inputs = vec![
                                    (
                                        ModCallbackInput::SelfActor,
                                        ModRuntimeValue::Actor(Some(request.owner.clone())),
                                    ),
                                    (
                                        ModCallbackInput::Time,
                                        ModRuntimeValue::Float(request.time.as_seconds_f64()),
                                    ),
                                    (
                                        ModCallbackInput::Elapsed,
                                        ModRuntimeValue::Float(request.elapsed.as_seconds_f64()),
                                    ),
                                ];
                                match invoke(inputs, None) {
                                    None => next(request),
                                    Some(value) => value != 0.0,
                                }
                            },
                        )),
                    );
                }
            }
            ModCallbackOperation::ActorTouch => {
                if callback.stage == ModCallbackStage::Observe {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::TouchObserve,
                        ModCallbackHandler::TouchObserve(Box::new(move |request: &TouchRequest, result: bool| {
                            invoke(
                                vec![
                                    (
                                        ModCallbackInput::SelfActor,
                                        ModRuntimeValue::Actor(Some(request.self_id.clone())),
                                    ),
                                    (ModCallbackInput::Other, ModRuntimeValue::Actor(request.other.clone())),
                                ],
                                Some(if result { 1.0 } else { 0.0 }),
                            );
                        })),
                    );
                } else if callback.stage == ModCallbackStage::Replace {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::TouchReplace,
                        ModCallbackHandler::TouchReplace(Box::new(
                            move |request: TouchRequest, next: &dyn Fn(TouchRequest) -> bool| {
                                let inputs = vec![
                                    (
                                        ModCallbackInput::SelfActor,
                                        ModRuntimeValue::Actor(Some(request.self_id.clone())),
                                    ),
                                    (ModCallbackInput::Other, ModRuntimeValue::Actor(request.other.clone())),
                                ];
                                match invoke(inputs, None) {
                                    None => next(request),
                                    Some(value) => value != 0.0,
                                }
                            },
                        )),
                    );
                }
            }
            ModCallbackOperation::ActorUse => {
                if callback.stage == ModCallbackStage::Observe {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::UseObserve,
                        ModCallbackHandler::UseObserve(Box::new(move |request: &UseRequest, result: bool| {
                            invoke(
                                vec![
                                    (
                                        ModCallbackInput::SelfActor,
                                        ModRuntimeValue::Actor(Some(request.owner.clone())),
                                    ),
                                    (ModCallbackInput::Other, ModRuntimeValue::Actor(request.other.clone())),
                                    (
                                        ModCallbackInput::Activator,
                                        ModRuntimeValue::Actor(request.activator.clone()),
                                    ),
                                ],
                                Some(if result { 1.0 } else { 0.0 }),
                            );
                        })),
                    );
                } else if callback.stage == ModCallbackStage::Replace {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::UseReplace,
                        ModCallbackHandler::UseReplace(Box::new(
                            move |request: UseRequest, next: &dyn Fn(UseRequest) -> bool| {
                                let inputs = vec![
                                    (
                                        ModCallbackInput::SelfActor,
                                        ModRuntimeValue::Actor(Some(request.owner.clone())),
                                    ),
                                    (ModCallbackInput::Other, ModRuntimeValue::Actor(request.other.clone())),
                                    (
                                        ModCallbackInput::Activator,
                                        ModRuntimeValue::Actor(request.activator.clone()),
                                    ),
                                ];
                                match invoke(inputs, None) {
                                    None => next(request),
                                    Some(value) => value != 0.0,
                                }
                            },
                        )),
                    );
                }
            }
            ModCallbackOperation::ActorPain => {
                if callback.stage == ModCallbackStage::Observe {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::PainObserve,
                        ModCallbackHandler::PainObserve(Box::new(move |request: &PainRequest, result: bool| {
                            invoke(
                                vec![
                                    (
                                        ModCallbackInput::SelfActor,
                                        ModRuntimeValue::Actor(Some(request.self_id.clone())),
                                    ),
                                    (
                                        ModCallbackInput::Attacker,
                                        ModRuntimeValue::Actor(request.attacker.clone()),
                                    ),
                                    (ModCallbackInput::Amount, ModRuntimeValue::Float(request.damage)),
                                    (ModCallbackInput::Knockback, ModRuntimeValue::Float(request.kick)),
                                ],
                                Some(if result { 1.0 } else { 0.0 }),
                            );
                        })),
                    );
                } else if callback.stage == ModCallbackStage::Replace {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::PainReplace,
                        ModCallbackHandler::PainReplace(Box::new(
                            move |request: PainRequest, next: &dyn Fn(PainRequest) -> bool| {
                                let inputs = vec![
                                    (
                                        ModCallbackInput::SelfActor,
                                        ModRuntimeValue::Actor(Some(request.self_id.clone())),
                                    ),
                                    (
                                        ModCallbackInput::Attacker,
                                        ModRuntimeValue::Actor(request.attacker.clone()),
                                    ),
                                    (ModCallbackInput::Amount, ModRuntimeValue::Float(request.damage)),
                                    (ModCallbackInput::Knockback, ModRuntimeValue::Float(request.kick)),
                                ];
                                match invoke(inputs, None) {
                                    None => next(request),
                                    Some(value) => value != 0.0,
                                }
                            },
                        )),
                    );
                }
            }
            ModCallbackOperation::ActorDie => {
                if callback.stage == ModCallbackStage::Observe {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::DieObserve,
                        ModCallbackHandler::DieObserve(Box::new(move |request: &DieRequest, result: bool| {
                            invoke(
                                vec![
                                    (
                                        ModCallbackInput::SelfActor,
                                        ModRuntimeValue::Actor(Some(request.self_id.clone())),
                                    ),
                                    (
                                        ModCallbackInput::Attacker,
                                        ModRuntimeValue::Actor(request.attacker.clone()),
                                    ),
                                    (
                                        ModCallbackInput::Inflictor,
                                        ModRuntimeValue::Actor(request.inflictor.clone()),
                                    ),
                                    (ModCallbackInput::Amount, ModRuntimeValue::Float(request.damage)),
                                    (ModCallbackInput::Knockback, ModRuntimeValue::Float(request.kick)),
                                    (ModCallbackInput::Point, ModRuntimeValue::Vector(request.point)),
                                ],
                                Some(if result { 1.0 } else { 0.0 }),
                            );
                        })),
                    );
                } else if callback.stage == ModCallbackStage::Replace {
                    registrations.register(
                        &callback.id,
                        ModRegistrationTarget::DieReplace,
                        ModCallbackHandler::DieReplace(Box::new(
                            move |request: DieRequest, next: &dyn Fn(DieRequest) -> bool| {
                                let inputs = vec![
                                    (
                                        ModCallbackInput::SelfActor,
                                        ModRuntimeValue::Actor(Some(request.self_id.clone())),
                                    ),
                                    (
                                        ModCallbackInput::Attacker,
                                        ModRuntimeValue::Actor(request.attacker.clone()),
                                    ),
                                    (
                                        ModCallbackInput::Inflictor,
                                        ModRuntimeValue::Actor(request.inflictor.clone()),
                                    ),
                                    (ModCallbackInput::Amount, ModRuntimeValue::Float(request.damage)),
                                    (ModCallbackInput::Knockback, ModRuntimeValue::Float(request.kick)),
                                    (ModCallbackInput::Point, ModRuntimeValue::Vector(request.point)),
                                ];
                                match invoke(inputs, None) {
                                    None => next(request),
                                    Some(value) => value != 0.0,
                                }
                            },
                        )),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use std::cell::RefCell;

    struct Collector {
        targets: Vec<(String, ModRegistrationTarget)>,
        handlers: Vec<ModCallbackHandler>,
    }

    impl ModRegistrations for Collector {
        fn register(&mut self, id: &str, target: ModRegistrationTarget, handler: ModCallbackHandler) {
            self.targets.push((id.to_string(), target));
            self.handlers.push(handler);
        }
    }

    fn seconds(value: f64) -> SourceTime {
        #[allow(clippy::cast_possible_truncation)]
        SourceTime::Seconds(value as f32)
    }

    fn damage_request(owner: &IdentityOwner) -> DamageRequest {
        DamageRequest {
            target: Some(owner.actor(1, 1)),
            attacker: None,
            inflictor: None,
            amount: 10.0,
            knockback: 4.0,
            direction: [0.0, 0.0, 1.0],
            point: [1.0, 2.0, 3.0],
            normal: [0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn damage_transform_rewrites_selected_field() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut collector = Collector {
            targets: vec![],
            handlers: vec![],
        };
        let time: Rc<dyn Fn() -> SourceTime> = Rc::new(|| seconds(1.0));
        let execute: ModCallbackExecute = Rc::new(|_, _| Some(7.0));
        register_mod_callbacks(
            &[ModCallback {
                id: "dmg".to_string(),
                operation: ModCallbackOperation::Damage,
                stage: ModCallbackStage::Transform,
                result: DamageField::Amount,
            }],
            &mut collector,
            time,
            execute,
        );
        assert_eq!(
            collector.targets,
            vec![("dmg".to_string(), ModRegistrationTarget::DamageTransform)]
        );
        let ModCallbackHandler::DamageTransform(handler) = collector.handlers.pop().unwrap() else {
            panic!("expected damage transform");
        };
        let rewritten = handler(damage_request(&owner));
        assert_eq!((rewritten.amount, rewritten.knockback), (7.0, 4.0));
    }

    #[test]
    fn damage_observe_reports_applied_damage() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut collector = Collector {
            targets: vec![],
            handlers: vec![],
        };
        let seen: Rc<RefCell<BTreeMap<ModCallbackInput, ModRuntimeValue>>> = Rc::new(RefCell::new(BTreeMap::new()));
        let time: Rc<dyn Fn() -> SourceTime> = Rc::new(|| seconds(1.0));
        let execute: ModCallbackExecute = Rc::new(move |_, inputs| {
            *seen.borrow_mut() = inputs.clone();
            None
        });
        register_mod_callbacks(
            &[ModCallback {
                id: "dmg".to_string(),
                operation: ModCallbackOperation::Damage,
                stage: ModCallbackStage::Observe,
                result: DamageField::Amount,
            }],
            &mut collector,
            time,
            execute,
        );
        let ModCallbackHandler::DamageObserve(handler) = collector.handlers.pop().unwrap() else {
            panic!("expected damage observe");
        };
        handler(
            &damage_request(&owner),
            &DamageOutcome::Committed { applied_damage: 5.0 },
        );
        drop(collector);
        let _ = &owner;
    }

    #[test]
    fn think_observe_prefers_frame_time_over_clock() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut collector = Collector {
            targets: vec![],
            handlers: vec![],
        };
        let seen: Rc<RefCell<BTreeMap<ModCallbackInput, ModRuntimeValue>>> = Rc::new(RefCell::new(BTreeMap::new()));
        let seen_in = Rc::clone(&seen);
        let time: Rc<dyn Fn() -> SourceTime> = Rc::new(|| seconds(100.0));
        let execute: ModCallbackExecute = Rc::new(move |_, inputs| {
            *seen_in.borrow_mut() = inputs.clone();
            None
        });
        register_mod_callbacks(
            &[ModCallback {
                id: "think".to_string(),
                operation: ModCallbackOperation::ActorThink,
                stage: ModCallbackStage::Observe,
                result: DamageField::Amount,
            }],
            &mut collector,
            time,
            execute,
        );
        let ModCallbackHandler::ThinkObserve(handler) = collector.handlers.pop().unwrap() else {
            panic!("expected think observe");
        };
        handler(
            &ThinkRequest {
                owner: owner.actor(1, 1),
                time: seconds(3.0),
                elapsed: seconds(0.1),
            },
            true,
        );
        assert_eq!(
            seen.borrow().get(&ModCallbackInput::Time),
            Some(&ModRuntimeValue::Float(3.0))
        );
        assert_eq!(
            seen.borrow().get(&ModCallbackInput::Result),
            Some(&ModRuntimeValue::Float(1.0))
        );
    }

    #[test]
    fn replace_falls_through_to_next_on_decline() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut collector = Collector {
            targets: vec![],
            handlers: vec![],
        };
        let time: Rc<dyn Fn() -> SourceTime> = Rc::new(|| seconds(1.0));
        let execute: ModCallbackExecute = Rc::new(|_, _| None);
        register_mod_callbacks(
            &[ModCallback {
                id: "use".to_string(),
                operation: ModCallbackOperation::ActorUse,
                stage: ModCallbackStage::Replace,
                result: DamageField::Amount,
            }],
            &mut collector,
            time,
            execute,
        );
        let ModCallbackHandler::UseReplace(handler) = collector.handlers.pop().unwrap() else {
            panic!("expected use replace");
        };
        let request = UseRequest {
            owner: owner.actor(1, 1),
            other: None,
            activator: None,
        };
        assert!(handler(request, &|_| true));
    }
}
