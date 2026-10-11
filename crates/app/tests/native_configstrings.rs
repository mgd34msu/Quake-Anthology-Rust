use qa_app::Runtime;
use qa_compat::{
    configstrings::NativeConfigs,
    memory::ModuleMemory,
    services::{CallError, ENGINE_CALLS, ServiceStorage},
};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::ModuleId;

#[test]
fn native_config_pointers_refresh_only_changed_published_values() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 3)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    let mut projection = NativeConfigs::load(0x10000, &[96, 8193, 96]).unwrap();
    let mut bytes = vec![0xa5; NativeConfigs::byte_length(&[96, 8193, 96]).unwrap()];
    let mut memory = ModuleMemory::borrow(0x10000, &mut bytes).unwrap();
    let first = projection
        .publish(services.storage, ModuleId(1), &mut memory, 0)
        .unwrap();
    assert_eq!(memory.cstring(first).unwrap(), b"");
    let revision = services.storage.config_revision(ModuleId(1)).unwrap();
    (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), 0, b"").unwrap();
    assert_eq!(
        services.storage.config_revision(ModuleId(1)).unwrap(),
        revision
    );
    (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), 0, b"\x80raw").unwrap();
    (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), 1, &vec![b'x'; 8192]).unwrap();
    projection
        .refresh(services.storage, ModuleId(1), &mut memory)
        .unwrap();
    assert_eq!(memory.cstring(first).unwrap(), b"\x80raw");
    assert_eq!(memory.read(first + 96, 1).unwrap(), &[0xa5]);
    let second = projection
        .publish(services.storage, ModuleId(1), &mut memory, 1)
        .unwrap();
    assert_eq!(memory.cstring(second).unwrap().len(), 8192);
    (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), 1, b"short").unwrap();
    projection
        .refresh(services.storage, ModuleId(1), &mut memory)
        .unwrap();
    assert_eq!(memory.cstring(second).unwrap(), b"short");
    assert_eq!(memory.cstring(first).unwrap(), b"\x80raw");
    assert_eq!(
        projection
            .publish(services.storage, ModuleId(1), &mut memory, 1)
            .unwrap(),
        second
    );
    assert_eq!(
        projection.publish(services.storage, ModuleId(1), &mut memory, 3),
        Err(CallError::ConfigString)
    );
    assert_eq!(
        projection.refresh(services.storage, ModuleId(2), &mut memory),
        Err(CallError::ConfigString)
    );
}

#[test]
fn native_config_projection_rejects_bad_layouts_and_extents() {
    for capacities in [&[][..], &[0][..], &[8194][..]] {
        assert!(matches!(
            NativeConfigs::load(0x10000, capacities),
            Err(CallError::Capacity)
        ));
    }
    assert!(matches!(
        NativeConfigs::load(u64::MAX, &[96]),
        Err(CallError::Memory)
    ));
    let console = Console::<Runtime>::new(Context::default()).unwrap();
    let storage = ServiceStorage::load(&[(ModuleId(1), 1)], 0, &console.cvars).unwrap();
    let mut projection = NativeConfigs::load(0x10000, &[96]).unwrap();
    let mut bytes = [];
    let mut memory = ModuleMemory::borrow(0x10000, &mut bytes).unwrap();
    assert_eq!(
        projection.publish(&storage, ModuleId(1), &mut memory, 0),
        Err(CallError::Memory)
    );
}

#[test]
fn retained_native_pointer_rebinds_to_new_storage_with_equal_revision() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut first_storage = ServiceStorage::load(&[(ModuleId(1), 1)], 0, &console.cvars).unwrap();
    let mut next_storage = ServiceStorage::load(&[(ModuleId(1), 1)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut projection = NativeConfigs::load(0x10000, &[96]).unwrap();
    let mut bytes = [0xa5; 96];
    let mut memory = ModuleMemory::borrow(0x10000, &mut bytes).unwrap();
    let mut services = runtime.engine_services(&mut console, &mut first_storage, &mut scratch);
    (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), 0, b"old").unwrap();
    let pointer = projection
        .publish(services.storage, ModuleId(1), &mut memory, 0)
        .unwrap();
    assert_eq!(memory.cstring(pointer).unwrap(), b"old");
    services.storage = &mut next_storage;
    (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), 0, b"new").unwrap();
    assert_eq!(services.storage.config_revision(ModuleId(1)).unwrap(), 1);
    projection.rebind();
    projection
        .refresh(services.storage, ModuleId(1), &mut memory)
        .unwrap();
    assert_eq!(memory.cstring(pointer).unwrap(), b"new");
    assert_eq!(
        projection
            .publish(services.storage, ModuleId(1), &mut memory, 0)
            .unwrap(),
        pointer
    );
}
