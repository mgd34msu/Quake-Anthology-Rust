use qa_core::primitives::{
    ClientId, EntityAppearance, EntityId, EntityPresentation, ModelRef, ModuleId, NativeClientRef,
    NativeEntityRef, NativeReference, ResourceNamespace, SoundAction, SoundEvent, SoundRef,
};

#[test]
fn native_model_and_sound_resources_keep_the_explicit_namespace_and_zero_sentinel() {
    for namespace in [
        ResourceNamespace::Module(ModuleId(17)),
        ResourceNamespace::Client(ClientId(3)),
    ] {
        assert_eq!(ModelRef::native(namespace, 0), None);
        assert_eq!(SoundRef::native(namespace, 0), None);
        let model = ModelRef::native(namespace, u32::MAX).unwrap();
        let sound = SoundRef::native(namespace, u32::MAX).unwrap();
        assert_eq!((model.namespace, model.ordinal), (namespace, u32::MAX));
        assert_eq!((sound.namespace, sound.ordinal), (namespace, u32::MAX));
        let appearance = EntityAppearance {
            models: [Some(model), None, None, None],
            loop_sound: Some(sound),
            ..EntityAppearance::default()
        };
        assert_eq!(appearance.loop_sound, Some(sound));
    }
    assert_eq!(EntityAppearance::default().loop_sound, None);
}

#[test]
fn loop_reset_and_client_retirement_retain_one_canonical_connection_scope() {
    let client = ClientId(19);
    let lifetime = EntityId {
        slot: 2,
        generation: 8,
    };
    for (event, action) in [
        (
            SoundEvent::reset_client_loops(client, lifetime),
            SoundAction::ResetClientLoops,
        ),
        (
            SoundEvent::retire_client(client, lifetime),
            SoundAction::RetireClient,
        ),
    ] {
        assert_eq!(event.action, action);
        assert_eq!(event.client, Some(client));
        assert_eq!(event.client_lifetime, Some(lifetime));
        assert_eq!(event.looping, None);
        assert!(event.applies_to_client(client));
        assert!(!event.applies_to_client(ClientId(20)));
    }
}

#[test]
fn native_entity_and_client_zero_are_explicit_distinct_weak_reference_roles() {
    let namespace = ResourceNamespace::Module(ModuleId(17));
    let entity = NativeReference::Entity(NativeEntityRef::entity(namespace, 0));
    let client = NativeReference::Client(NativeClientRef::client(namespace, 0));
    assert_ne!(entity, client);
    assert_eq!(entity.namespace(), namespace);
    assert_eq!(client.namespace(), namespace);
    assert_eq!(entity.ordinal(), 0);
    assert_eq!(client.ordinal(), 0);
    let presentation = EntityPresentation::new(namespace);
    assert_eq!(presentation.namespace, namespace);
    assert_eq!(presentation.ground, None);
    assert_eq!(presentation.other_entity2, None);
    let empty = EntityAppearance::default();
    assert_eq!(empty.presentation, None);
    assert_eq!(empty.native.other_entity, None);
}
