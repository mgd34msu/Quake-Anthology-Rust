//! Owned import stops yield all byte borrows before independent host work.
use super::{BASE, DATA, REGIONS, function, image_child, imports, put, word};
use crate::native::{
    NativeAbi, NativeCallToken, NativeError, NativeImportToken, NativeProcess, NativeScalar,
    NativeStop,
};
use std::time::Duration;

fn import(process: &mut NativeProcess, token: &NativeCallToken, number: u32) -> NativeImportToken {
    let NativeStop::Import { call, stopped } = process.advance(token).unwrap() else {
        panic!("expected suspended import")
    };
    assert_eq!(call.number, number);
    stopped
}
// Deliberate controller replays cannot be constructed by a public caller.
fn replay_import(token: &NativeImportToken) -> NativeImportToken {
    NativeImportToken {
        owner: token.owner,
        transaction: token.transaction,
        frame: token.frame,
        import: token.import,
    }
}
fn replay_call(token: &NativeCallToken) -> NativeCallToken {
    NativeCallToken {
        owner: token.owner,
        transaction: token.transaction,
        frame: token.frame,
    }
}

#[test]
fn raw_stops_allow_external_host_work_and_nested_exports_under_the_current_park() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let mut declarations = imports(abi);
        declarations[0].result = NativeScalar::I8;
        declarations[1].result = NativeScalar::Double;
        let mut bytes = vec![0; 8192];
        let first = NativeProcess::import_address(BASE, bytes.len(), 0).unwrap();
        let second = NativeProcess::import_address(BASE, bytes.len(), 1).unwrap();
        put(&mut bytes, 0, &function(&[], first, &[]));
        let mut before = vec![];
        word(&mut before, &[0x48, 0xba], DATA);
        before.extend([
            0x48, 0x8b, 0x42, 64, 0x48, 0x83, 0xc0, 1, 0x48, 0x89, 0x42, 72,
        ]);
        put(&mut bytes, 256, &function(&before, second, &[]));
        put(&mut bytes, 512, &[0xb8, 77, 0, 0, 0, 0xc3]);
        let mut process =
            image_child(&bytes, &REGIONS, &declarations, Duration::from_secs(3)).unwrap();
        let mut external =
            image_child(&bytes, &REGIONS, &declarations, Duration::from_secs(3)).unwrap();
        let outer_entry = process.bind(BASE, abi, &[], NativeScalar::Word).unwrap();
        let inner_entry = process
            .bind(BASE + 256, abi, &[], NativeScalar::Double)
            .unwrap();
        let external_entry = external
            .bind(BASE + 512, abi, &[], NativeScalar::Word)
            .unwrap();
        let outer = process.enter(outer_entry, [0; 13]).unwrap();
        let stopped = import(&mut process, &outer, 10);
        assert!(matches!(process.memory(), Err(NativeError::Protocol)));
        assert!(matches!(process.memory_mut(), Err(NativeError::Protocol)));
        process
            .with_import_memory(&stopped, |base, memory| {
                assert_eq!(base, BASE);
                memory[4160..4168].copy_from_slice(&111u64.to_le_bytes());
                Ok(())
            })
            .unwrap();
        // No A/services/mapped-byte borrow is held while another owner runs.
        let host_step = external.enter(external_entry, [0; 13]).unwrap();
        assert!(matches!(
            external.advance(&host_step).unwrap(),
            NativeStop::Returned(77)
        ));
        let nested = process.enter(inner_entry, [0; 13]).unwrap();
        let inner_stop = import(&mut process, &nested, 11);
        process
            .with_import_memory(&inner_stop, |_, memory| {
                assert_eq!(
                    u64::from_le_bytes(memory[4168..4176].try_into().unwrap()),
                    112
                );
                Ok(())
            })
            .unwrap();
        process.reply(inner_stop, 5.5f64.to_bits()).unwrap();
        assert!(
            matches!(process.advance(&nested).unwrap(), NativeStop::Returned(value) if value == 5.5f64.to_bits())
        );
        // This logical token outlived two newer kernel notifications. Only the
        // process's current validated nested-RETURN park authorizes this loan.
        process
            .with_import_memory(&stopped, |_, memory| {
                assert_eq!(
                    u64::from_le_bytes(memory[4168..4176].try_into().unwrap()),
                    112
                );
                Ok(())
            })
            .unwrap();
        process.reply(stopped, 255).unwrap();
        assert!(matches!(
            process.advance(&outer).unwrap(),
            NativeStop::Returned(u64::MAX)
        ));
        assert!(process.memory_mut().is_ok());
    }
}

#[test]
fn wrong_expired_and_non_top_tokens_do_not_borrow_unpark_or_cancel_an_owner() {
    let abi = NativeAbi::SystemV;
    let declarations = imports(abi);
    let mut bytes = vec![0; 8192];
    let first = NativeProcess::import_address(BASE, bytes.len(), 0).unwrap();
    put(&mut bytes, 0, &function(&[], first, &[]));
    put(&mut bytes, 512, &[0xb8, 77, 0, 0, 0, 0xc3]);
    let mut process = image_child(&bytes, &REGIONS, &declarations, Duration::from_secs(3)).unwrap();
    let mut peer = image_child(&bytes, &REGIONS, &declarations, Duration::from_secs(3)).unwrap();
    let entry = process.bind(BASE, abi, &[], NativeScalar::Word).unwrap();
    let healthy = process
        .bind(BASE + 512, abi, &[], NativeScalar::Word)
        .unwrap();
    let peer_entry = peer.bind(BASE, abi, &[], NativeScalar::Word).unwrap();
    let root = process.enter(entry, [0; 13]).unwrap();
    let root_stop = import(&mut process, &root, 10);
    let peer_root = peer.enter(peer_entry, [0; 13]).unwrap();
    let peer_stop = import(&mut peer, &peer_root, 10);
    assert!(matches!(
        process.with_import_memory::<()>(&peer_stop, |_, _| panic!("foreign loan")),
        Err(NativeError::Protocol)
    ));
    assert!(matches!(
        process.reply(replay_import(&peer_stop), 0),
        Err(NativeError::Protocol)
    ));
    assert!(matches!(
        process.abort(replay_call(&peer_root), NativeError::Callback),
        NativeError::Protocol
    ));
    let nested = process.enter(entry, [0; 13]).unwrap();
    assert!(matches!(process.advance(&root), Err(NativeError::Protocol)));
    assert!(matches!(
        process.with_import_memory::<()>(&root_stop, |_, _| panic!("ancestor loan")),
        Err(NativeError::Protocol)
    ));
    assert!(matches!(
        process.reply(replay_import(&root_stop), 0),
        Err(NativeError::Protocol)
    ));
    let nested_stop = import(&mut process, &nested, 10);
    let expired_import = replay_import(&nested_stop);
    process.reply(nested_stop, 0).unwrap();
    assert!(matches!(
        process.advance(&nested).unwrap(),
        NativeStop::Returned(_)
    ));
    assert!(matches!(
        process.with_import_memory::<()>(&expired_import, |_, _| panic!("expired loan")),
        Err(NativeError::Protocol)
    ));
    assert!(matches!(
        process.advance(&nested),
        Err(NativeError::Protocol)
    ));
    process.reply(root_stop, 0).unwrap();
    assert!(matches!(
        process.advance(&root).unwrap(),
        NativeStop::Returned(_)
    ));
    let fresh = process.enter(healthy, [0; 13]).unwrap();
    assert!(matches!(process.advance(&root), Err(NativeError::Protocol)));
    assert!(matches!(
        process.advance(&fresh).unwrap(),
        NativeStop::Returned(77)
    ));
    peer.reply(peer_stop, 0).unwrap();
    assert!(matches!(
        peer.advance(&peer_root).unwrap(),
        NativeStop::Returned(_)
    ));
    assert_ne!(process.pid(), 0);
    assert_ne!(peer.pid(), 0);
}

#[test]
fn external_work_and_nested_entry_share_the_original_deadline() {
    let abi = NativeAbi::SystemV;
    let declarations = imports(abi);
    let mut bytes = vec![0; 8192];
    let first = NativeProcess::import_address(BASE, bytes.len(), 0).unwrap();
    put(&mut bytes, 0, &function(&[], first, &[]));
    put(&mut bytes, 512, &[0xb8, 77, 0, 0, 0, 0xc3]);
    let mut process = image_child(&bytes, &REGIONS, &declarations, Duration::from_secs(1)).unwrap();
    let outer_entry = process.bind(BASE, abi, &[], NativeScalar::Word).unwrap();
    let nested_entry = process
        .bind(BASE + 512, abi, &[], NativeScalar::Word)
        .unwrap();
    let outer = process.enter(outer_entry, [0; 13]).unwrap();
    let stopped = import(&mut process, &outer, 10);
    crate::pause(Duration::from_millis(600));
    let nested = process.enter(nested_entry, [0; 13]).unwrap();
    assert!(matches!(
        process.advance(&nested).unwrap(),
        NativeStop::Returned(77)
    ));
    crate::pause(Duration::from_millis(500));
    assert!(matches!(
        process.reply(stopped, 0),
        Err(NativeError::Timeout)
    ));
    assert_eq!(process.pid(), 0);
    assert!(matches!(
        process.advance(&outer),
        Err(NativeError::Protocol)
    ));
    assert!(process.memory_mut().is_ok());
}

#[test]
fn real_child_trap_invalidates_all_stops_but_a_fake_host_trap_reaps() {
    let abi = NativeAbi::Microsoft;
    let mut declarations = imports(abi);
    declarations[1].trap = true;
    declarations[1].result = NativeScalar::Void;
    let mut bytes = vec![0; 8192];
    let first = NativeProcess::import_address(BASE, bytes.len(), 0).unwrap();
    let trap = NativeProcess::import_address(BASE, bytes.len(), 1).unwrap();
    put(&mut bytes, 0, &function(&[], first, &[]));
    put(&mut bytes, 256, &function(&[], trap, &[0x0f, 0x0b]));
    put(&mut bytes, 512, &[0xb8, 77, 0, 0, 0, 0xc3]);
    let mut process = image_child(&bytes, &REGIONS, &declarations, Duration::from_secs(3)).unwrap();
    let pid = process.pid();
    let root_entry = process.bind(BASE, abi, &[], NativeScalar::Word).unwrap();
    let trap_entry = process
        .bind(BASE + 256, abi, &[], NativeScalar::Word)
        .unwrap();
    let healthy_entry = process
        .bind(BASE + 512, abi, &[], NativeScalar::Word)
        .unwrap();
    let root = process.enter(root_entry, [0; 13]).unwrap();
    let stopped = import(&mut process, &root, 10);
    let nested = process.enter(trap_entry, [0; 13]).unwrap();
    assert!(matches!(
        process.advance(&nested),
        Err(NativeError::ImportTrap { ordinal: 1, .. })
    ));
    assert!(matches!(process.advance(&root), Err(NativeError::Protocol)));
    assert!(matches!(
        process.with_import_memory::<()>(&stopped, |_, _| panic!("trapped loan")),
        Err(NativeError::Protocol)
    ));
    assert_eq!(process.pid(), pid);
    let healthy = process.enter(healthy_entry, [0; 13]).unwrap();
    assert!(matches!(
        process.advance(&healthy).unwrap(),
        NativeStop::Returned(77)
    ));
    let root = process.enter(root_entry, [0; 13]).unwrap();
    let stopped = import(&mut process, &root, 10);
    let result: Result<(), _> = process.with_import_memory(&stopped, |_, _| {
        Err(NativeError::ImportTrap {
            ordinal: 1,
            address: 0,
        })
    });
    assert!(matches!(result, Err(NativeError::ImportTrap { .. })));
    assert_eq!(process.pid(), 0);
    assert!(matches!(process.advance(&root), Err(NativeError::Protocol)));
}

#[test]
fn canceling_an_ancestor_reaps_the_transaction_and_invalidates_nested_stops() {
    let abi = NativeAbi::SystemV;
    let declarations = imports(abi);
    let mut bytes = vec![0; 8192];
    let first = NativeProcess::import_address(BASE, bytes.len(), 0).unwrap();
    put(&mut bytes, 0, &function(&[], first, &[]));
    let mut process = image_child(&bytes, &REGIONS, &declarations, Duration::from_secs(3)).unwrap();
    let entry = process.bind(BASE, abi, &[], NativeScalar::Word).unwrap();
    let root = process.enter(entry, [0; 13]).unwrap();
    let root_stop = import(&mut process, &root, 10);
    let nested = process.enter(entry, [0; 13]).unwrap();
    let nested_stop = import(&mut process, &nested, 10);
    assert!(matches!(
        process.abort(root, NativeError::Callback),
        NativeError::Callback
    ));
    assert_eq!(process.pid(), 0);
    assert!(matches!(
        process.advance(&nested),
        Err(NativeError::Protocol)
    ));
    assert!(matches!(
        process.with_import_memory::<()>(&root_stop, |_, _| panic!("canceled ancestor")),
        Err(NativeError::Protocol)
    ));
    assert!(matches!(
        process.with_import_memory::<()>(&nested_stop, |_, _| panic!("canceled nested")),
        Err(NativeError::Protocol)
    ));
}
