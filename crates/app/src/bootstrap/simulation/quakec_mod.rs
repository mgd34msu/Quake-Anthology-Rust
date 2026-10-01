//! QuakeC gameplay-mod host: validated declaration plus prepared media over the guest provider.
//!
//! Ported from donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/quakec-mod.ts`.
//!
//! Donor mapping:
//! - `QuakeCModParams` (media/identity subset) becomes [`QuakeCModConfig`]. The donor params also
//!   carry render and HUD adapters owned by other partitions; those stay out of this module.
//! - `QuakeCMod` becomes [`QuakeCMod`], which owns the guest [`QcModProvider`] plus the media
//!   owner. Lifecycle, invocation, projections, media lookup, checkpoints, travel identity, and
//!   client/match presentation all forward to the provider.
//! - `QuakeCModCallbackInputs` becomes [`QuakeCModCallbackInputs`] ([`QcModInputs`]); the provider
//!   resolves declared `ModCallbackValue` expressions against these runtime values per call.
//! - `QuakeCModMediaOwner` becomes the [`QuakeCModMediaOwner`] trait seam: the owner is a
//!   host-provided value, so it is injected, never duplicated. The donor `damage` accessor
//!   resolves through the declaration combat lowering ([`QuakeCMod::combat`]); the donor `mod`
//!   accessor is the owned provider itself ([`QuakeCMod::provider`]).
//! - Donor `identity` becomes [`QuakeCMod::identity`], including the verbatim `providers: []`
//!   (donor line 38). The donor composes `{ guests: [source.checkpoint()], providers: [] }` at the
//!   session layer, so this module exposes [`QuakeCMod::checkpoint`] (the guest payload) and leaves
//!   `ModPrivateCheckpoint` composition to that layer, which owns the checkpoint format strings.

use std::collections::HashMap;

use qa_core::identity::{ActorId, ProviderId, SavedActorId};
use qa_core::time::{FrameContext, SourceTime};
use qa_guest::error::GuestError;
use qa_guest::qc::mod_provider::{
    ClientRelease, ContentDigest, ModCallback, ModCallbackDeclaration, ModCombatDeclaration, ModCvar, ModIdentity,
    ModQcItems, ModQcProtection, ModSelection, ModSourceCall, ModuleIdentity, ProviderReference, QcClientFrame,
    QcMatchPlayer, QcMediaResource, QcModCheckpoint, QcModInputs, QcModMedia, QcModProvider, QcModelPresentation,
    QcObjectiveDeclaration, QcPrecachedResource, QcProviderMachine, QcProviderServices, SourceCallValidator,
};

/// Host callback inputs for one mod invocation.
///
/// Donor `QuakeCModCallbackInputs` from `quakec-mod.ts`: a partial map from callback input slot to
/// runtime value. The canonical home is [`QcModInputs`]; this alias keeps the donor name visible.
pub type QuakeCModCallbackInputs = QcModInputs;

/// Prepared-media owner seam.
///
/// Donor `QuakeCModMediaOwner` from `quakec-mod.ts` is a host-provided value (the prepared content
/// owner), so per the cross-partition rule it is injected as a trait, never duplicated. The owner
/// keeps the authoritative prepared resources; the provider holds its own media copy for lookups.
pub trait QuakeCModMediaOwner {
    /// Authoritative prepared resources by name.
    fn resources(&self) -> &HashMap<String, QcMediaResource>;
    /// Bind the live module and provider after the provider validates.
    fn attach_mod(&mut self, module: &ModuleIdentity, provider: &ProviderId);
    /// Channel the owner serves owned-mod traffic on.
    fn owned_mod_channel(&self) -> &str;
    /// Restore owner state at a mod checkpoint boundary.
    fn restore(&mut self, checkpoint: &QcModCheckpoint) -> Result<(), GuestError>;
    /// Release owner state.
    fn close(&mut self);
}

/// Open parameters for [`QuakeCMod`].
///
/// Donor `QuakeCModParams` media/identity subset from `quakec-mod.ts`.
#[derive(Debug, Clone)]
pub struct QuakeCModConfig {
    /// Module identity for the gameplay program.
    pub module: ModuleIdentity,
    /// Provider identity for this mod instance.
    pub provider: ProviderId,
    /// Validated callback declaration.
    pub declaration: ModCallbackDeclaration,
    /// Prepared media; required when the declaration carries source items.
    pub media: Option<QcModMedia>,
    /// Mod selection identity for travel checkpoints.
    pub selection: ModSelection,
    /// Content source reference for travel checkpoints.
    pub source: ProviderReference,
}

/// QuakeC gameplay mod: validated declaration plus prepared media over the guest provider.
///
/// The donor `QuakeCMod` class from `quakec-mod.ts`.
pub struct QuakeCMod<M, S, O> {
    provider: QcModProvider<M, S>,
    selection: ModSelection,
    source: ProviderReference,
    owner: O,
}

impl<M: QcProviderMachine, S: QcProviderServices, O: QuakeCModMediaOwner> QuakeCMod<M, S, O> {
    /// Open a mod: validate the declaration, then attach the media owner.
    ///
    /// `validate_call` is the caller-provided source-call policy. When provider construction fails
    /// the owner is left untouched.
    pub fn open(
        machine: M,
        services: S,
        config: QuakeCModConfig,
        mut owner: O,
        validate_call: &SourceCallValidator<'_>,
    ) -> Result<Self, GuestError> {
        let module = config.module.clone();
        let provider_id = config.provider.clone();
        let provider = QcModProvider::new(
            machine,
            services,
            module,
            provider_id,
            config.declaration,
            config.media,
            validate_call,
        )?;
        owner.attach_mod(&config.module, &config.provider);
        Ok(Self {
            provider,
            selection: config.selection,
            source: config.source,
            owner,
        })
    }

    /// Borrow the guest provider (donor `mod` accessor).
    #[must_use]
    pub fn provider(&self) -> &QcModProvider<M, S> {
        &self.provider
    }

    /// Mutably borrow the guest provider.
    pub fn provider_mut(&mut self) -> &mut QcModProvider<M, S> {
        &mut self.provider
    }

    /// Borrow the machine.
    #[must_use]
    pub fn machine(&self) -> &M {
        self.provider.machine()
    }

    /// Mutably borrow the machine.
    pub fn machine_mut(&mut self) -> &mut M {
        self.provider.machine_mut()
    }

    /// Borrow the host services.
    #[must_use]
    pub fn services(&self) -> &S {
        self.provider.services()
    }

    /// Mutably borrow the host services.
    pub fn services_mut(&mut self) -> &mut S {
        self.provider.services_mut()
    }

    /// Borrow the media owner.
    #[must_use]
    pub fn owner(&self) -> &O {
        &self.owner
    }

    /// Mutably borrow the media owner.
    pub fn owner_mut(&mut self) -> &mut O {
        &mut self.owner
    }

    /// Module identity.
    #[must_use]
    pub fn module(&self) -> &ModuleIdentity {
        self.provider.module()
    }

    /// Provider identity.
    #[must_use]
    pub fn provider_id(&self) -> &ProviderId {
        self.provider.provider()
    }

    /// Validated callback declaration.
    #[must_use]
    pub fn declaration(&self) -> &ModCallbackDeclaration {
        self.provider.declaration()
    }

    /// Mod selection identity.
    #[must_use]
    pub fn selection(&self) -> &ModSelection {
        &self.selection
    }

    /// Content source reference.
    #[must_use]
    pub fn source(&self) -> &ProviderReference {
        &self.source
    }

    /// Current nested invocation depth.
    #[must_use]
    pub fn depth(&self) -> u32 {
        self.provider.depth()
    }

    /// Whether initialization ran.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.provider.is_initialized()
    }

    /// Whether the mod is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.provider.is_closed()
    }

    /// Presentation generation, bumped on restore.
    #[must_use]
    pub fn presentation_generation(&self) -> u64 {
        self.provider.presentation_generation()
    }

    /// Run the declared initialization calls once at an idle boundary.
    pub fn initialize(&mut self) -> Result<(), GuestError> {
        self.provider.initialize()
    }

    /// Advance one frame: run the frame call, then advance owned actors.
    pub fn advance(&mut self, frame: &FrameContext) -> Result<(), GuestError> {
        self.provider.advance(frame)
    }

    /// Close the provider and release the media owner.
    pub fn close(&mut self) {
        self.provider.close();
        self.owner.close();
    }

    /// Invoke a source call with host inputs.
    pub fn invoke(&mut self, call: &ModSourceCall, inputs: &QuakeCModCallbackInputs) -> Result<f64, GuestError> {
        self.provider.invoke(call, inputs)
    }

    /// Invoke a no-argument owned-actor callback.
    pub fn invoke_owned(
        &mut self,
        index: i32,
        actor: &ActorId,
        other: Option<&ActorId>,
        time: &SourceTime,
    ) -> Result<(), GuestError> {
        self.provider.invoke_owned(index, actor, other, time)
    }

    /// Dispatch a console invocation; reports whether a command matched.
    pub fn console_command(&mut self, argv: &[String], args_text: &str) -> Result<bool, GuestError> {
        self.provider.console_command(argv, args_text)
    }

    /// Resolve an entity reference to its projected actor.
    pub fn actor(&self, reference: i32) -> Result<ActorId, GuestError> {
        self.provider.actor(reference)
    }

    /// Project an actor to an entity reference, allocating a slot when new.
    pub fn reference(&mut self, actor: Option<&ActorId>) -> Result<i32, GuestError> {
        self.provider.reference(actor)
    }

    /// Release one client projection, deferring while a callback runs.
    pub fn release_client_projection(&mut self, actor: &ActorId) -> Result<ClientRelease, GuestError> {
        self.provider.release_client_projection(actor)
    }

    /// Retire one actor projection.
    pub fn retire_projection(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        self.provider.retire_projection(actor)
    }

    /// Drain retired projections at an idle boundary.
    pub fn drain_retired_projections(&mut self) -> Result<(), GuestError> {
        self.provider.drain_retired_projections()
    }

    /// Look up (and precache) a prepared resource by kind and name.
    pub fn lookup(&mut self, kind: &str, name: &str) -> Option<QcPrecachedResource> {
        self.provider.lookup(kind, name)
    }

    /// Capture the provider checkpoint (donor `source.checkpoint()` payload).
    #[must_use]
    pub fn checkpoint(&self) -> QcModCheckpoint {
        self.provider.checkpoint()
    }

    /// Restore provider projections and owner state at an idle callback boundary.
    pub fn restore(
        &mut self,
        saved: &QcModCheckpoint,
        resolve: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    ) -> Result<(), GuestError> {
        self.provider.restore(saved, resolve)?;
        self.owner.restore(saved)
    }

    /// Travel identity for this mod instance (donor `identity`, `providers: []` verbatim).
    #[must_use]
    pub fn identity(&self, declaration_digest: ContentDigest) -> ModIdentity {
        ModIdentity {
            selection: self.selection.clone(),
            source: self.source.clone(),
            declaration_digest,
            modules: vec![self.provider.module().clone()],
            providers: Vec::new(),
        }
    }

    /// Model presentations for owned actors.
    pub fn presentations(&self) -> Result<Vec<QcModelPresentation>, GuestError> {
        self.provider.presentations()
    }

    /// Client frame for one admitted actor, when the declaration publishes one.
    pub fn client_frame(&self, actor: &ActorId) -> Result<Option<QcClientFrame>, GuestError> {
        self.provider.client_frame(actor)
    }

    /// Activate match services.
    pub fn activate_match(&mut self) -> Result<(), GuestError> {
        self.provider.activate_match()
    }

    /// Match row for one actor.
    pub fn match_player(&self, actor: &ActorId) -> Result<Option<QcMatchPlayer>, GuestError> {
        self.provider.match_player(actor)
    }

    /// Set one actor's match team.
    pub fn set_match_team(&mut self, actor: &ActorId, team: Option<String>) -> Result<(), GuestError> {
        self.provider.set_match_team(actor, team)
    }

    /// Set one actor's match score.
    pub fn set_match_score(&mut self, actor: &ActorId, score: f64) -> Result<(), GuestError> {
        self.provider.set_match_score(actor, score)
    }

    /// Combat lowering (donor `damage` accessor resolves through here).
    #[must_use]
    pub fn combat(&self) -> Option<&ModCombatDeclaration> {
        self.provider.declaration().combat.as_ref()
    }

    /// Protection channels.
    #[must_use]
    pub fn protection(&self) -> &[ModQcProtection] {
        &self.provider.declaration().protection
    }

    /// Source items declaration, when present.
    #[must_use]
    pub fn items(&self) -> Option<&ModQcItems> {
        self.provider.declaration().items.as_ref()
    }

    /// Declared callbacks.
    #[must_use]
    pub fn callbacks(&self) -> &[ModCallback] {
        &self.provider.declaration().callbacks
    }

    /// Declared console variables.
    #[must_use]
    pub fn cvars(&self) -> &[ModCvar] {
        &self.provider.declaration().cvars
    }

    /// Declared objectives.
    #[must_use]
    pub fn objectives(&self) -> &[QcObjectiveDeclaration] {
        &self.provider.declaration().objectives
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, OwnedActor};
    use qa_core::math::Vec3;
    use qa_core::time::FramePhase;
    use qa_guest::qc::mod_provider::{
        ItemAdmission, ModActorBinding, ModActorField, ModCallbackInput, ModCallbackValue, ModClientDeclaration,
        ModCombatDeclaration, ModConsoleCommand, ModConsoleValue, ModItemCapacity, ModItemDefinition, ModItemKind,
        ModItemStorage, ModProgramRef, ModQcItems, ModRuntimeValue, ModSourceGlobal, QcApiKind, QcFunctionView,
        QcProgramView, QcValueType,
    };

    #[derive(Debug, Clone, PartialEq)]
    struct StubCall {
        function: String,
        arguments: Vec<ModRuntimeValue>,
        globals: Vec<(String, ModRuntimeValue)>,
    }

    struct StubView {
        functions: Vec<String>,
        float_fields: Vec<String>,
    }

    impl StubView {
        fn view(&self, name: &str, index: usize) -> QcFunctionView {
            QcFunctionView {
                index: index as i32 + 1,
                name: name.to_string(),
                first_statement: 0,
                parameter_start: 0,
                parameter_sizes: Vec::new(),
                named_builtin: false,
            }
        }
    }

    impl QcProgramView for StubView {
        fn digest(&self) -> &str {
            "test-digest"
        }

        fn api_kind(&self) -> QcApiKind {
            QcApiKind::Q1Quakeworld
        }

        fn field_type(&self, name: &str) -> Option<QcValueType> {
            self.float_fields
                .iter()
                .any(|field| field == name)
                .then_some(QcValueType::Float)
        }

        fn global_type(&self, _name: &str) -> Option<QcValueType> {
            None
        }

        fn function_named(&self, name: &str) -> Option<QcFunctionView> {
            let index = self.functions.iter().position(|candidate| candidate == name)?;
            let found = self.functions[index].clone();
            Some(self.view(&found, index))
        }

        fn function_at(&self, index: i32) -> Option<QcFunctionView> {
            if index < 1 {
                return None;
            }
            let position = usize::try_from(index - 1).ok()?;
            let found = self.functions.get(position)?.clone();
            Some(self.view(&found, position))
        }

        fn functions(&self) -> Vec<QcFunctionView> {
            self.functions
                .iter()
                .enumerate()
                .map(|(index, name)| self.view(name, index))
                .collect()
        }
    }

    struct StubMachine {
        view: StubView,
        entity_slots: u32,
        result: f64,
        calls: Vec<StubCall>,
    }

    impl QcProviderMachine for StubMachine {
        fn program(&self) -> &dyn QcProgramView {
            &self.view
        }

        fn field_offset(&self, name: &str) -> Result<i32, GuestError> {
            Err(GuestError::invalid(format!("unknown test field {name}")))
        }

        fn global_offset(&self, name: &str) -> Result<i32, GuestError> {
            Err(GuestError::invalid(format!("unknown test global {name}")))
        }

        fn entity_count(&self) -> u32 {
            self.entity_slots
        }

        fn set_entity_count(&mut self, count: u32) -> Result<(), GuestError> {
            self.entity_slots = count;
            Ok(())
        }

        fn entity_slot(&self, reference: i32) -> Result<u32, GuestError> {
            if reference < 1 {
                return Err(GuestError::invalid("test reference out of range"));
            }
            Ok((reference - 1) as u32)
        }

        fn entity_reference(&self, slot: u32) -> i32 {
            slot as i32 + 1
        }

        fn zero_slot(&mut self, _slot: u32) -> Result<(), GuestError> {
            Ok(())
        }

        fn slot_float(&self, _slot: u32, _offset: i32) -> Result<f32, GuestError> {
            Ok(0.0)
        }

        fn set_slot_float(&mut self, _slot: u32, _offset: i32, _value: f32) -> Result<(), GuestError> {
            Ok(())
        }

        fn slot_vector(&self, _slot: u32, _offset: i32) -> Result<Vec3, GuestError> {
            Ok(Vec3::default())
        }

        fn set_slot_vector(&mut self, _slot: u32, _offset: i32, _value: Vec3) -> Result<(), GuestError> {
            Ok(())
        }

        fn slot_int(&self, _slot: u32, _offset: i32) -> Result<i32, GuestError> {
            Ok(0)
        }

        fn set_slot_int(&mut self, _slot: u32, _offset: i32, _value: i32) -> Result<(), GuestError> {
            Ok(())
        }

        fn strings_get(&self, index: i32) -> Result<String, GuestError> {
            Err(GuestError::invalid(format!("unknown test string {index}")))
        }

        fn strings_allocate(&mut self, _text: &str) -> Result<i32, GuestError> {
            Err(GuestError::invalid("test strings are read-only"))
        }

        fn set_global_float(&mut self, _name: &str, _value: f32) -> Result<(), GuestError> {
            Ok(())
        }

        fn invoke_resolved(
            &mut self,
            function: &str,
            arguments: &[ModRuntimeValue],
            globals: &[(String, ModRuntimeValue)],
        ) -> Result<f64, GuestError> {
            self.calls.push(StubCall {
                function: function.to_string(),
                arguments: arguments.to_vec(),
                globals: globals.to_vec(),
            });
            Ok(self.result)
        }
    }

    struct StubServices {
        now: SourceTime,
        owned: Vec<OwnedActor>,
        published: usize,
        flushed_messages: usize,
        advanced: Vec<FrameContext>,
    }

    impl QcProviderServices for StubServices {
        fn now(&self) -> SourceTime {
            self.now
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.owned.iter().any(|owned| owned.id() == actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.owned.iter().find(|owned| owned.id() == actor).cloned()
        }

        fn world_actor(&self) -> Option<ActorId> {
            None
        }

        fn publish_client_outputs(&mut self) {
            self.published += 1;
        }

        fn flush_messages(&mut self) {
            self.flushed_messages += 1;
        }

        fn advance_owned_actors(&mut self, frame: &FrameContext) -> Result<(), GuestError> {
            self.advanced.push(*frame);
            Ok(())
        }
    }

    #[derive(Default)]
    struct StubOwner {
        resources: HashMap<String, QcMediaResource>,
        attached: Vec<(ModuleIdentity, ProviderId)>,
        restores: usize,
        closed: bool,
    }

    impl QuakeCModMediaOwner for StubOwner {
        fn resources(&self) -> &HashMap<String, QcMediaResource> {
            &self.resources
        }

        fn attach_mod(&mut self, module: &ModuleIdentity, provider: &ProviderId) {
            self.attached.push((module.clone(), provider.clone()));
        }

        fn owned_mod_channel(&self) -> &str {
            "test-channel"
        }

        fn restore(&mut self, _checkpoint: &QcModCheckpoint) -> Result<(), GuestError> {
            self.restores += 1;
            Ok(())
        }

        fn close(&mut self) {
            self.closed = true;
        }
    }

    fn accept_all(_call: &ModSourceCall, _inputs: &[ModCallbackInput], _context: &str) -> Result<(), GuestError> {
        Ok(())
    }

    fn test_module() -> ModuleIdentity {
        ModuleIdentity {
            id: "test-module".to_string(),
            artifact_path: "progs.dat".to_string(),
            digest: "module-digest".to_string(),
            revision: 1,
        }
    }

    fn test_config(mut declaration: ModCallbackDeclaration, media: Option<QcModMedia>) -> QuakeCModConfig {
        declaration.program = Some(ModProgramRef {
            path: "progs.dat".to_string(),
            digest: "test-digest".to_string(),
        });
        QuakeCModConfig {
            module: test_module(),
            provider: ProviderId::new("test", "qc-mod"),
            declaration,
            media,
            selection: ModSelection {
                product: "test-product".to_string(),
                id: "test-mod".to_string(),
            },
            source: ProviderReference {
                provider: ProviderId::new("test", "content"),
                content: "content-id".to_string(),
            },
        }
    }

    fn test_machine(functions: &[&str]) -> StubMachine {
        machine_with_fields(functions, &[])
    }

    fn machine_with_fields(functions: &[&str], fields: &[&str]) -> StubMachine {
        StubMachine {
            view: StubView {
                functions: functions.iter().map(ToString::to_string).collect(),
                float_fields: fields.iter().map(ToString::to_string).collect(),
            },
            entity_slots: 0,
            result: 0.0,
            calls: Vec::new(),
        }
    }

    fn test_clients() -> ModClientDeclaration {
        ModClientDeclaration {
            outputs: Vec::new(),
            maximum: 16,
            admit: Vec::new(),
            userinfo: Vec::new(),
            disconnect: Vec::new(),
            frame: Vec::new(),
            input: Vec::new(),
        }
    }

    fn test_services() -> StubServices {
        StubServices {
            now: SourceTime::Seconds(1.5),
            owned: Vec::new(),
            published: 0,
            flushed_messages: 0,
            advanced: Vec::new(),
        }
    }

    fn open_mod(
        declaration: ModCallbackDeclaration,
        machine: StubMachine,
        services: StubServices,
        media: Option<QcModMedia>,
    ) -> QuakeCMod<StubMachine, StubServices, StubOwner> {
        QuakeCMod::open(
            machine,
            services,
            test_config(declaration, media),
            StubOwner::default(),
            &accept_all,
        )
        .expect("test mod opens")
    }

    fn bare_call(function: &str) -> ModSourceCall {
        ModSourceCall {
            function: function.to_string(),
            arguments: Vec::new(),
            globals: Vec::new(),
        }
    }

    #[test]
    fn open_attaches_owner_and_reports_identity() {
        let gama = open_mod(
            ModCallbackDeclaration::default(),
            test_machine(&[]),
            test_services(),
            None,
        );
        assert_eq!(gama.module(), &test_module());
        assert_eq!(gama.provider_id(), &ProviderId::new("test", "qc-mod"));
        assert_eq!(gama.selection().id, "test-mod");
        assert_eq!(gama.source().content, "content-id");
        assert_eq!(gama.depth(), 0);
        assert_eq!(gama.presentation_generation(), 0);
        assert!(!gama.is_initialized());
        assert!(!gama.is_closed());
        assert!(gama.combat().is_none());
        assert!(gama.items().is_none());
        assert!(gama.callbacks().is_empty());
        assert!(gama.cvars().is_empty());
        assert!(gama.objectives().is_empty());
        assert!(gama.protection().is_empty());
        assert_eq!(
            gama.owner().attached,
            vec![(test_module(), ProviderId::new("test", "qc-mod"))]
        );
        assert_eq!(gama.owner().owned_mod_channel(), "test-channel");
        assert!(gama.owner().resources().is_empty());
    }

    fn valid_items() -> ModQcItems {
        ModQcItems {
            definitions: vec![ModItemDefinition {
                item: "q1:item_shells".to_string(),
                label: "Shells".to_string(),
                icon: None,
                admission: ItemAdmission::Add,
                kind: ModItemKind::Counter,
                actions: None,
            }],
            storage: vec![ModItemStorage::Counter {
                field: "ammo_shells".to_string(),
                item: "q1:item_shells".to_string(),
                capacity: ModItemCapacity::Constant { value: 100.0 },
            }],
            weapons: None,
        }
    }

    #[test]
    fn open_rejects_source_items_without_media() {
        let declaration = ModCallbackDeclaration {
            items: Some(valid_items()),
            ..Default::default()
        };
        let result = QuakeCMod::open(
            test_machine(&[]),
            test_services(),
            test_config(declaration, None),
            StubOwner::default(),
            &accept_all,
        );
        assert!(result.is_err());
    }

    #[test]
    fn initialize_runs_declared_calls_once() {
        let declaration = ModCallbackDeclaration {
            initialize: vec![bare_call("init_fn")],
            ..Default::default()
        };
        let mut gama = open_mod(declaration, test_machine(&["init_fn"]), test_services(), None);
        gama.initialize().expect("initialize runs");
        assert!(gama.is_initialized());
        assert_eq!(gama.machine().calls.len(), 1);
        assert_eq!(gama.machine().calls[0].function, "init_fn");
        assert!(gama.machine().calls[0].arguments.is_empty());
        assert!(gama.initialize().is_err());
    }

    #[test]
    fn advance_runs_frame_call_and_advances_owners() {
        let declaration = ModCallbackDeclaration {
            frame: Some(bare_call("frame_fn")),
            ..Default::default()
        };
        let mut gama = open_mod(declaration, test_machine(&["frame_fn"]), test_services(), None);
        let frame = FrameContext {
            frame: 3,
            time: SourceTime::Seconds(0.1),
            elapsed: SourceTime::Seconds(0.016),
            phase: FramePhase::FrameEntry,
        };
        gama.advance(&frame).expect("advance runs");
        assert_eq!(gama.machine().calls.len(), 1);
        assert_eq!(gama.machine().calls[0].function, "frame_fn");
        assert_eq!(gama.services().advanced, vec![frame]);
    }

    #[test]
    fn invoke_resolves_inputs_through_machine() {
        let mut machine = test_machine(&[]);
        machine.result = 7.0;
        let mut gama = open_mod(ModCallbackDeclaration::default(), machine, test_services(), None);
        let call = ModSourceCall {
            function: "any_fn".to_string(),
            arguments: vec![
                ModCallbackValue::Input(ModCallbackInput::Time),
                ModCallbackValue::Float(1.5),
            ],
            globals: vec![ModSourceGlobal {
                name: "self".to_string(),
                value: ModCallbackValue::Input(ModCallbackInput::Self_),
            }],
        };
        let mut inputs = QuakeCModCallbackInputs::new();
        inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(2.0));
        inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(None));
        assert_eq!(gama.invoke(&call, &inputs).expect("invoke runs"), 7.0);
        let recorded = &gama.machine().calls;
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].arguments,
            vec![ModRuntimeValue::Float(2.0), ModRuntimeValue::Float(1.5)]
        );
        assert_eq!(
            recorded[0].globals,
            vec![("self".to_string(), ModRuntimeValue::Actor(None))]
        );
        assert_eq!(gama.services().published, 1);
        assert_eq!(gama.services().flushed_messages, 1);
        assert_eq!(gama.depth(), 0);
    }

    #[test]
    fn invoke_rejects_closed_mod() {
        let mut gama = open_mod(
            ModCallbackDeclaration::default(),
            test_machine(&[]),
            test_services(),
            None,
        );
        gama.close();
        assert!(gama.is_closed());
        assert!(gama.owner().closed);
        assert!(gama
            .invoke(&bare_call("any_fn"), &QuakeCModCallbackInputs::new())
            .is_err());
    }

    #[test]
    fn console_command_dispatches_declared_command() {
        let declaration = ModCallbackDeclaration {
            commands: vec![ModConsoleCommand {
                name: "fire".to_string(),
                function: "Cmd_Fire".to_string(),
                arguments: vec![ModConsoleValue::Float(3.0)],
                globals: Vec::new(),
            }],
            ..Default::default()
        };
        let mut gama = open_mod(declaration, test_machine(&["Cmd_Fire"]), test_services(), None);
        let matched = gama.console_command(&["FIRE".to_string()], "").expect("dispatch runs");
        assert!(matched);
        assert_eq!(gama.machine().calls.len(), 1);
        assert_eq!(gama.machine().calls[0].function, "Cmd_Fire");
        assert_eq!(gama.machine().calls[0].arguments, vec![ModRuntimeValue::Float(3.0)]);
        let missed = gama
            .console_command(&["unknown".to_string()], "")
            .expect("dispatch runs");
        assert!(!missed);
    }

    #[test]
    fn checkpoint_restores_projections() {
        let identity = IdentityOwner::create("test").expect("identity owner");
        let actor = identity.actor(3, 1);
        let mut services = test_services();
        services.owned = vec![identity
            .owned_actor(&actor, ProviderId::new("test", "qc-mod"))
            .expect("owned actor")];
        let mut gama = open_mod(ModCallbackDeclaration::default(), test_machine(&[]), services, None);
        let reference = gama.reference(Some(&actor)).expect("reference projects");
        assert_eq!(reference, 1);
        assert_eq!(gama.actor(reference).expect("actor resolves"), actor);
        let saved = gama.checkpoint();
        assert_eq!(saved.projections.len(), 1);

        let mut services = test_services();
        services.owned = vec![identity
            .owned_actor(&actor, ProviderId::new("test", "qc-mod"))
            .expect("owned actor")];
        let mut machine = test_machine(&[]);
        machine.entity_slots = 1;
        let mut restored = open_mod(ModCallbackDeclaration::default(), machine, services, None);
        let expect = saved.projections[0].0;
        let resolve = |saved: &SavedActorId| (saved == &expect).then(|| actor.clone());
        restored.restore(&saved, &resolve).expect("restore runs");
        assert_eq!(restored.checkpoint(), saved);
        assert_eq!(restored.owner().restores, 1);
        assert_eq!(restored.presentation_generation(), 1);
    }

    #[test]
    fn restore_rejects_unresolvable_saved_actor() {
        let mut gama = open_mod(
            ModCallbackDeclaration::default(),
            test_machine(&[]),
            test_services(),
            None,
        );
        let saved = QcModCheckpoint {
            projections: vec![(SavedActorId { slot: 9, generation: 0 }, 0)],
            precached: Vec::new(),
            initialized: false,
        };
        assert!(gama.restore(&saved, &|_| None).is_err());
    }

    #[test]
    fn lookup_precaches_prepared_media() {
        let declaration = ModCallbackDeclaration {
            clients: Some(test_clients()),
            actor_fields: vec![ModActorField {
                field: "ammo_shells".to_string(),
                binding: ModActorBinding::Private,
            }],
            items: Some(valid_items()),
            ..Default::default()
        };
        let mut resources = HashMap::new();
        resources.insert(
            "progs/plasma.mdl".to_string(),
            QcMediaResource {
                requested_path: "progs/plasma.mdl".to_string(),
                model_bounds: None,
            },
        );
        let media = QcModMedia {
            content: "content-id".to_string(),
            resources,
        };
        let mut gama = open_mod(
            declaration,
            machine_with_fields(&[], &["ammo_shells"]),
            test_services(),
            Some(media),
        );
        assert!(gama.items().is_some());
        let first = gama.lookup("model", "progs/plasma.mdl").expect("resource resolves");
        assert_eq!(first.index, 1);
        assert_eq!(first.requested_path, "progs/plasma.mdl");
        let again = gama
            .lookup("model", "progs/plasma.mdl")
            .expect("cached resource resolves");
        assert_eq!(again, first);
        assert!(gama.lookup("model", "missing.mdl").is_none());
        let saved = gama.checkpoint();
        assert_eq!(saved.precached, vec!["model:progs/plasma.mdl".to_string()]);
    }

    #[test]
    fn identity_carries_selection_module_and_empty_providers() {
        let gama = open_mod(
            ModCallbackDeclaration::default(),
            test_machine(&[]),
            test_services(),
            None,
        );
        let identity = gama.identity("declaration-digest".to_string());
        assert_eq!(identity.selection.id, "test-mod");
        assert_eq!(identity.source.content, "content-id");
        assert_eq!(identity.declaration_digest, "declaration-digest");
        assert_eq!(identity.modules, vec![test_module()]);
        assert!(identity.providers.is_empty());
    }

    #[test]
    fn declaration_views_expose_lowering_slices() {
        let declaration = ModCallbackDeclaration {
            combat: Some(ModCombatDeclaration {
                damage: bare_call("damage_fn"),
                damage_scale: None,
                armor_stage: None,
                empty_armor: None,
            }),
            cvars: vec![ModCvar {
                name: "test_cvar".to_string(),
                value: "1".to_string(),
            }],
            ..Default::default()
        };
        let gama = open_mod(
            declaration,
            machine_with_fields(
                &["damage_fn"],
                &[
                    "health",
                    "takedamage",
                    "flags",
                    "invincible_finished",
                    "armorvalue",
                    "armortype",
                ],
            ),
            test_services(),
            None,
        );
        assert_eq!(gama.combat().expect("combat lowering").damage.function, "damage_fn");
        assert_eq!(gama.cvars().len(), 1);
        assert_eq!(gama.cvars()[0].name, "test_cvar");
    }
}
