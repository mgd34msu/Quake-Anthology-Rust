//! Cgame body submissions: authored `refEntity` capture and suppression.
//!
//! Provenance: `src/compat/qvm/cgame-body.ts`.
//!
//! [`QvmBodyPart`] comes from [`super::mod_presentation`] (which absorbs
//! `src/contracts/qvm-mod-presentation.ts`); [`QvmSceneBodyMesh`] is a local
//! mirror of the donor mesh declaration (no like-named donor export exists).
//! Reference entities reuse [`super::render_record`]. The donor requires
//! bytecode artifacts; mirror artifacts are always bytecode, so that check
//! is vacuous here.
//!
//! Hook closures cannot fail, so unresolvable guest words proceed with the
//! original call instead of unwinding across an interpreter boundary. Scope
//! disorder (unreachable in single-threaded mirror use) is recorded for
//! [`QvmBodySubmissions::take_scope_error`] instead of throwing mid-unwind.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::game_data::{
    qualify_qvm_body_calls, CallKind, QvmArtifact, QvmCgameImport, QvmFunctionCall, QvmHookFn, QvmHostCall, QvmModule,
    QvmOpcode, QvmRole, QVM_MAX_PRIVATE_ARGUMENT_WORDS,
};
use super::mod_presentation::QvmBodyPart;
use super::render_record::{read_qvm_ref_entity, QvmRefEntity, QvmRefEntityKind, QVM_REF_ENTITY_BYTES};
use crate::error::GuestError;

/// One qualified mesh call row: call site plus its body part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmSceneBodyMeshCall {
    /// Mesh call site.
    pub call: usize,
    /// Body part submitted at the site.
    pub part: QvmBodyPart,
}

/// Scene body mesh declaration: mesh entry plus its argument layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmSceneBodyMesh {
    /// Mesh function entry.
    pub entry: usize,
    /// Argument word holding the entity pointer.
    pub entity_argument: i32,
    /// Argument word holding the state word.
    pub state_argument: i32,
    /// Shader field offset within the entity.
    pub shader_offset: i32,
    /// Explicit call rows, if any.
    pub parts: Option<Vec<QvmSceneBodyMeshCall>>,
}

/// Reference-entity storage addressed by a body submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmBodyReference {
    /// The authored body lives in the caller frame locals.
    Locals,
    /// The authored body lives behind an argument pointer.
    Argument {
        /// Argument word holding the pointer.
        index: usize,
    },
}

/// Submission guard: only run when an argument word equals a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmBodyWhen {
    /// Argument word to test.
    pub argument: usize,
    /// Required value.
    pub equals: i32,
}

/// Declared body submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmBodySubmission {
    /// Player function entry.
    pub entry: usize,
    /// Argument word holding the centity pointer.
    pub actor_argument: usize,
    /// Entity-number field offset within the centity.
    pub entity_number_offset: usize,
    /// Reference-entity storage.
    pub reference: QvmBodyReference,
    /// Submission guard, if any.
    pub when: Option<QvmBodyWhen>,
    /// Qualified mesh submission, if any.
    pub mesh: Option<QvmSceneBodyMesh>,
}

/// Body capture callbacks.
pub trait QvmBodyCapture {
    /// Whether the entity is selected for capture.
    fn selected(&self, entity: i32) -> bool;
    /// Submit a captured model part; returns whether the scene trap is consumed.
    fn submit(&self, entity: i32, part: &QvmBodyPart, source: &QvmRefEntity, base: bool) -> bool;
}

#[derive(Debug, Clone)]
struct BodyRange {
    id: u64,
    start: usize,
    end: usize,
    entity: i32,
    state: i32,
    hidden: bool,
    declaration: usize,
    calls: Vec<(usize, QvmBodyPart)>,
}

#[derive(Debug, Clone)]
struct BodyMesh {
    id: u64,
    range: u64,
    pointer: i32,
    part: QvmBodyPart,
    shader: i32,
}

#[derive(Debug, Default)]
struct BodyShared {
    ranges: Vec<BodyRange>,
    meshes: Vec<BodyMesh>,
    removals: Vec<u64>,
    next_id: u64,
    scope_error: Option<String>,
}

/// Authored body submissions over a cgame module.
#[derive(Clone)]
pub struct QvmBodySubmissions {
    module: QvmModule,
    artifact: QvmArtifact,
    declarations: Vec<QvmBodySubmission>,
    hidden: Rc<dyn Fn(i32) -> bool>,
    capture: Option<Rc<dyn QvmBodyCapture>>,
    calls: HashMap<usize, Vec<(usize, QvmBodyPart)>>,
    shared: Rc<RefCell<BodyShared>>,
}

impl std::fmt::Debug for QvmBodySubmissions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QvmBodySubmissions")
            .field("declarations", &self.declarations.len())
            .field("ranges", &self.shared.borrow().ranges.len())
            .finish()
    }
}

impl QvmBodySubmissions {
    /// Declare body submissions, qualifying mesh calls up front.
    pub fn new(
        module: &QvmModule,
        artifact: &QvmArtifact,
        declarations: Vec<QvmBodySubmission>,
        hidden: Rc<dyn Fn(i32) -> bool>,
        capture: Option<Rc<dyn QvmBodyCapture>>,
    ) -> Result<Self, GuestError> {
        if artifact.role != QvmRole::Cgame {
            return Err(GuestError::invalid("Body submissions require authored cgame bytecode"));
        }
        let mut calls: HashMap<usize, Vec<(usize, QvmBodyPart)>> = HashMap::new();
        let mut seen = HashSet::new();
        for declaration in &declarations {
            let instruction = artifact.image.instruction(declaration.entry);
            if instruction.is_none_or(|instruction| {
                instruction.opcode != QvmOpcode::OpEnter || instruction.operand < 8
            }) || !seen.insert(declaration.entry)
            {
                return Err(GuestError::invalid(
                    "Cgame body declaration requires a unique source function entry",
                ));
            }
            let mut arguments = vec![declaration.actor_argument];
            if let Some(when) = &declaration.when {
                arguments.push(when.argument);
            }
            if let QvmBodyReference::Argument { index } = &declaration.reference {
                arguments.push(*index);
            }
            if arguments
                .iter()
                .any(|argument| *argument >= QVM_MAX_PRIVATE_ARGUMENT_WORDS)
            {
                return Err(GuestError::invalid("Cgame body argument is outside the source ABI"));
            }
            if declaration.entity_number_offset % 4 != 0 {
                return Err(GuestError::invalid(
                    "Cgame body entity number requires an aligned field offset",
                ));
            }
            let qualified = match &declaration.mesh {
                None => Vec::new(),
                Some(mesh) => {
                    let parts: Vec<(usize, QvmBodyPart)> = mesh
                        .parts
                        .as_ref()
                        .map(|parts| parts.iter().map(|row| (row.call, row.part)).collect())
                        .unwrap_or_default();
                    let explicit = mesh.parts.is_some();
                    qualify_qvm_body_calls(
                        &artifact.image,
                        declaration.entry,
                        declaration.actor_argument as i32,
                        mesh.entry,
                        mesh.entity_argument,
                        mesh.state_argument,
                        mesh.shader_offset,
                        if explicit { Some(parts.as_slice()) } else { None },
                        QvmBodyPart::Body,
                    )?
                }
            };
            calls.insert(declaration.entry, qualified);
        }
        Ok(Self {
            module: module.clone(),
            artifact: artifact.clone(),
            declarations,
            hidden,
            capture,
            calls,
            shared: Rc::new(RefCell::new(BodyShared::default())),
        })
    }

    /// Whether any declaration captures player meshes.
    #[must_use]
    pub fn captures_player_meshes(&self) -> bool {
        self.declarations.iter().any(|declaration| declaration.mesh.is_some())
    }

    /// Take a recorded scope-disorder error, if any.
    #[must_use]
    pub fn take_scope_error(&self) -> Option<String> {
        self.shared.borrow_mut().scope_error.take()
    }

    /// Enable or disable submission hooks.
    pub fn enable(&self, active: bool) -> Result<(), GuestError> {
        if !self.shared.borrow().ranges.is_empty() {
            return Err(GuestError::invalid(
                "Cannot change body submissions during source rendering",
            ));
        }
        if !active {
            for id in self.shared.borrow_mut().removals.drain(..) {
                self.module.remove_hook(id);
            }
            return Ok(());
        }
        if !self.shared.borrow().removals.is_empty() {
            return Ok(());
        }
        for declaration in &self.declarations {
            let instruction = self.artifact.image.instruction(declaration.entry);
            if instruction.is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter) {
                return Err(GuestError::invalid("Cgame body entry changed"));
            }
            if !self.calls.contains_key(&declaration.entry) {
                return Err(GuestError::invalid("Cgame body declaration lost its qualified calls"));
            }
        }
        let mut mesh_entries = HashSet::new();
        for (index, declaration) in self.declarations.iter().enumerate() {
            let frame_bytes = self
                .artifact
                .image
                .instruction(declaration.entry)
                .map(|instruction| instruction.operand.max(0) as usize)
                .unwrap_or(0);
            let hook: QvmHookFn = {
                let submissions = self.clone();
                let declaration = declaration.clone();
                Rc::new(move |call| submissions.player_hook(call, index, &declaration, frame_bytes))
            };
            let id = self.module.bind_function(declaration.entry, hook);
            self.shared.borrow_mut().removals.push(id);
            if let Some(mesh) = &declaration.mesh {
                if mesh_entries.insert(mesh.entry) {
                    let hook: QvmHookFn = {
                        let submissions = self.clone();
                        let mesh = mesh.clone();
                        Rc::new(move |call| submissions.mesh_hook(call, &mesh))
                    };
                    let id = self.module.bind_function(mesh.entry, hook);
                    self.shared.borrow_mut().removals.push(id);
                }
            }
        }
        Ok(())
    }

    fn player_hook(
        &self,
        call: &mut QvmFunctionCall,
        index: usize,
        declaration: &QvmBodySubmission,
        frame_bytes: usize,
    ) -> i32 {
        if let Some(when) = &declaration.when {
            if call.argument(when.argument).unwrap_or(0) != when.equals {
                return call.proceed();
            }
        }
        let actor_word = call.argument(declaration.actor_argument).unwrap_or(0);
        let entity = usize::try_from(actor_word)
            .ok()
            .and_then(|base| base.checked_add(declaration.entity_number_offset))
            .and_then(|address| call.guest.read_i32(address).ok());
        let Some(entity) = entity else {
            return call.proceed();
        };
        let hidden = (self.hidden)(entity);
        let selected = self.capture.as_ref().is_some_and(|capture| capture.selected(entity));
        if !hidden && !selected {
            return call.proceed();
        }
        if selected && declaration.mesh.is_none() && !hidden {
            return call.proceed();
        }
        let (start, end) = match &declaration.reference {
            QvmBodyReference::Locals => {
                let stack = call.stack_address.checked_sub(8);
                let start = stack.and_then(|stack| stack.checked_sub(frame_bytes));
                let (Some(stack), Some(start)) = (stack, start) else {
                    return call.proceed();
                };
                (start, stack)
            }
            QvmBodyReference::Argument { index } => {
                let start = call
                    .argument(*index)
                    .ok()
                    .and_then(|word| usize::try_from(word).ok())
                    .and_then(|offset| {
                        offset
                            .checked_add(QVM_REF_ENTITY_BYTES)
                            .filter(|end| *end <= call.guest.len())
                            .map(|_| offset)
                    });
                let Some(start) = start else {
                    return call.proceed();
                };
                (start, start + QVM_REF_ENTITY_BYTES)
            }
        };
        let mut shared = self.shared.borrow_mut();
        shared.next_id += 1;
        let range = BodyRange {
            id: shared.next_id,
            start,
            end,
            entity,
            state: actor_word.wrapping_add(declaration.entity_number_offset as i32),
            hidden,
            declaration: index,
            calls: self.calls.get(&declaration.entry).cloned().unwrap_or_default(),
        };
        let id = range.id;
        shared.ranges.push(range);
        drop(shared);
        let result = call.proceed();
        let mut shared = self.shared.borrow_mut();
        if shared.ranges.pop().map(|range| range.id) != Some(id) {
            shared.scope_error = Some("Cgame body submission scopes unwound out of order".to_string());
        }
        result
    }

    fn mesh_hook(&self, call: &mut QvmFunctionCall, mesh: &QvmSceneBodyMesh) -> i32 {
        let active = (|| {
            let shared = self.shared.borrow();
            let range = shared.ranges.last()?;
            let caller = call.caller_instruction?;
            let part = range.calls.iter().find(|(site, _)| *site == caller)?.1;
            if self.declarations.get(range.declaration)?.mesh.as_ref()?.entry != mesh.entry {
                return None;
            }
            if call.argument(mesh.state_argument as usize).unwrap_or(0) != range.state {
                return None;
            }
            let pointer = call.argument(mesh.entity_argument as usize).unwrap_or(0);
            let shader = usize::try_from(pointer)
                .ok()
                .and_then(|base| base.checked_add(mesh.shader_offset as usize))
                .and_then(|address| call.guest.read_i32(address).ok())?;
            Some((range.id, pointer, part, shader))
        })();
        let Some((range, pointer, part, shader)) = active else {
            return call.proceed();
        };
        let mut shared = self.shared.borrow_mut();
        shared.next_id += 1;
        let id = shared.next_id;
        shared.meshes.push(BodyMesh {
            id,
            range,
            pointer,
            part,
            shader,
        });
        drop(shared);
        let result = call.proceed();
        let mut shared = self.shared.borrow_mut();
        if shared.meshes.pop().map(|mesh| mesh.id) != Some(id) {
            shared.scope_error = Some("Cgame body mesh scopes unwound out of order".to_string());
        }
        result
    }

    /// Suppress or capture an `ADDREFENTITYTOSCENE` trap.
    pub fn suppress(&self, call: &QvmHostCall) -> bool {
        if self.shared.borrow().ranges.is_empty()
            || call.kind != CallKind::Engine
            || call.role != QvmRole::Cgame
            || call.code != QvmCgameImport::CG_R_ADDREFENTITYTOSCENE
        {
            return false;
        }
        let pointer = call.int(1).unwrap_or(0);
        let start = usize::try_from(pointer).unwrap_or(usize::MAX);
        let (range, part) = {
            let shared = self.shared.borrow();
            let range = shared
                .ranges
                .iter()
                .rev()
                .find(|range| start >= range.start && start.saturating_add(QVM_REF_ENTITY_BYTES) <= range.end);
            let Some(range) = range else {
                return false;
            };
            (range.clone(), shared.meshes.last().cloned())
        };
        if !range.hidden
            && part
                .as_ref()
                .is_some_and(|part| part.range == range.id && part.pointer == pointer)
            && self
                .capture
                .as_ref()
                .is_some_and(|capture| capture.selected(range.entity))
        {
            let bytes = call.guest.read_bytes(start, QVM_REF_ENTITY_BYTES);
            if let Ok(bytes) = bytes {
                if let Ok(source) = read_qvm_ref_entity(&bytes) {
                    if let Some(part) = part.as_ref() {
                        if source.kind == QvmRefEntityKind::Model
                            && self.capture.as_ref().is_some_and(|capture| {
                                capture.submit(range.entity, &part.part, &source, source.custom_shader == part.shader)
                            })
                        {
                            return true;
                        }
                    }
                }
            }
        }
        range.hidden
    }

    /// Disable submission hooks.
    pub fn close(&self) -> Result<(), GuestError> {
        self.enable(false)
    }
}

#[cfg(test)]
mod tests {
    use super::super::game_data::{QvmImage, QvmInstruction};
    use super::*;

    struct FixtureCapture {
        submitted: RefCell<Vec<(i32, QvmBodyPart, bool)>>,
    }

    impl QvmBodyCapture for FixtureCapture {
        fn selected(&self, entity: i32) -> bool {
            entity == 7
        }

        fn submit(&self, entity: i32, part: &QvmBodyPart, source: &QvmRefEntity, base: bool) -> bool {
            assert_eq!(source.kind, QvmRefEntityKind::Model);
            self.submitted.borrow_mut().push((entity, *part, base));
            true
        }
    }

    fn image() -> QvmImage {
        QvmImage {
            instructions: vec![
                QvmInstruction::word(QvmOpcode::OpEnter, 64, 0),
                QvmInstruction::word(QvmOpcode::OpConst, 4, 5),
                QvmInstruction::single(QvmOpcode::OpCall, 10),
                QvmInstruction::single(QvmOpcode::OpLeave, 11),
                QvmInstruction::word(QvmOpcode::OpEnter, 16, 12),
            ],
            allocated_data_length: 4096,
            ..Default::default()
        }
    }

    fn submissions(hidden_entity: i32) -> (QvmBodySubmissions, Rc<FixtureCapture>) {
        let artifact = QvmArtifact {
            module: super::super::game_data::ModuleIdentity {
                id: "q3:cgame".to_string(),
                artifact_path: "cgame.qvm".to_string(),
                digest: "d".to_string(),
                revision: "r".to_string(),
            },
            role: QvmRole::Cgame,
            abi_profile: None,
            image: image(),
        };
        let module = QvmModule::new(artifact.clone(), None, None).unwrap();
        let capture = Rc::new(FixtureCapture {
            submitted: RefCell::new(Vec::new()),
        });
        let capture_hook: Rc<dyn QvmBodyCapture> = capture.clone();
        let declarations = vec![QvmBodySubmission {
            entry: 0,
            actor_argument: 0,
            entity_number_offset: 0,
            reference: QvmBodyReference::Argument { index: 1 },
            when: None,
            mesh: Some(QvmSceneBodyMesh {
                entry: 4,
                entity_argument: 0,
                state_argument: 1,
                shader_offset: 112,
                parts: None,
            }),
        }];
        let submissions = QvmBodySubmissions::new(
            &module,
            &artifact,
            declarations,
            Rc::new(move |entity| entity == hidden_entity),
            Some(capture_hook),
        )
        .unwrap();
        (submissions, capture)
    }

    fn trap(submissions: &QvmBodySubmissions, pointer: i32) -> QvmHostCall {
        QvmHostCall {
            kind: CallKind::Engine,
            role: QvmRole::Cgame,
            code: QvmCgameImport::CG_R_ADDREFENTITYTOSCENE,
            words: vec![QvmCgameImport::CG_R_ADDREFENTITYTOSCENE, pointer],
            guest: submissions.module.memory(),
            abi_profile: super::super::game_data::AbiProfile::Modern,
            command_arguments: None,
        }
    }

    #[test]
    fn declarations_validate_entries_and_arguments() {
        let (submissions, _) = submissions(7);
        assert!(submissions.captures_player_meshes());
        let artifact = submissions.artifact.clone();
        let module = submissions.module.clone();
        let hidden: Rc<dyn Fn(i32) -> bool> = Rc::new(|_| false);
        let bad_entry = vec![QvmBodySubmission {
            entry: 2,
            actor_argument: 0,
            entity_number_offset: 0,
            reference: QvmBodyReference::Locals,
            when: None,
            mesh: None,
        }];
        assert!(QvmBodySubmissions::new(&module, &artifact, bad_entry, hidden.clone(), None).is_err());
        let bad_argument = vec![QvmBodySubmission {
            entry: 0,
            actor_argument: QVM_MAX_PRIVATE_ARGUMENT_WORDS,
            entity_number_offset: 0,
            reference: QvmBodyReference::Locals,
            when: None,
            mesh: None,
        }];
        assert!(QvmBodySubmissions::new(&module, &artifact, bad_argument, hidden.clone(), None).is_err());
        let unaligned = vec![QvmBodySubmission {
            entry: 0,
            actor_argument: 0,
            entity_number_offset: 2,
            reference: QvmBodyReference::Locals,
            when: None,
            mesh: None,
        }];
        assert!(QvmBodySubmissions::new(&module, &artifact, unaligned, hidden, None).is_err());
    }

    #[test]
    fn hooks_balance_and_gate_on_hidden_or_selected() {
        let (submissions, _) = submissions(7);
        submissions.enable(true).unwrap();
        submissions.enable(true).unwrap();
        let memory = submissions.module.memory();
        memory.write_i32(100, 7).unwrap();
        memory.write_i32(300, 9).unwrap();
        assert_eq!(submissions.module.call(&[100, 200], 0).unwrap(), 0);
        assert!(submissions.shared.borrow().ranges.is_empty());
        assert!(submissions.take_scope_error().is_none());
        let calls = submissions.module.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].words, vec![100, 200]);
        submissions.close().unwrap();
        assert!(submissions.module.call(&[100, 200], 0).is_ok());
    }

    #[test]
    fn suppress_captures_model_parts_and_hides_hidden_ranges() {
        let (submissions, capture) = submissions(9);
        let memory = submissions.module.memory();
        let mut model = vec![0u8; QVM_REF_ENTITY_BYTES];
        model[112..116].copy_from_slice(&5i32.to_le_bytes());
        memory.write_bytes(200, &model).unwrap();
        submissions.shared.borrow_mut().ranges.push(BodyRange {
            id: 1,
            start: 200,
            end: 340,
            entity: 7,
            state: 100,
            hidden: false,
            declaration: 0,
            calls: vec![(2, QvmBodyPart::Body)],
        });
        submissions.shared.borrow_mut().meshes.push(BodyMesh {
            id: 2,
            range: 1,
            pointer: 200,
            part: QvmBodyPart::Body,
            shader: 5,
        });
        let call = trap(&submissions, 200);
        assert!(submissions.suppress(&call));
        assert_eq!(capture.submitted.borrow().as_slice(), &[(7, QvmBodyPart::Body, true)]);
        model[0..4].copy_from_slice(&1i32.to_le_bytes());
        memory.write_bytes(200, &model).unwrap();
        assert!(!submissions.suppress(&call));
        assert_eq!(capture.submitted.borrow().len(), 1);
        submissions.shared.borrow_mut().ranges[0].hidden = true;
        assert!(submissions.suppress(&call));
        submissions.shared.borrow_mut().ranges.clear();
        assert!(!submissions.suppress(&call));
    }

    #[test]
    fn enable_refuses_changes_during_rendering() {
        let (submissions, _) = submissions(7);
        submissions.shared.borrow_mut().ranges.push(BodyRange {
            id: 1,
            start: 0,
            end: 140,
            entity: 1,
            state: 0,
            hidden: false,
            declaration: 0,
            calls: Vec::new(),
        });
        assert!(submissions.enable(true).is_err());
        assert!(submissions.close().is_err());
        submissions.shared.borrow_mut().ranges.clear();
        submissions.close().unwrap();
    }
}
