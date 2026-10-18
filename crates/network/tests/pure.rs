use qa_core::primitives::CommandArgs;
use qa_network::pure::{Admission, Decision, PACKAGE_LIMIT, Policy};

fn policy(feed: u32) -> Policy {
    Policy::new(feed, [Some(0xffff_ff80), Some(91)], [7, 0xffff_fffe, 19])
}
fn verify(admission: &mut Admission, text: &str) -> Decision {
    let args: Vec<_> = text.split_ascii_whitespace().map(str::as_bytes).collect();
    admission.verify(CommandArgs {
        argv: &args,
        raw: text.as_bytes(),
    })
}
#[test]
fn native_signed_words_delimiter_prefix_zero_refs_and_outdated_commands() {
    let mut state = Admission::default();
    assert_eq!(verify(&mut state, "cp"), Decision::Ignored);
    assert!(state.allows_movement());
    assert!(!state.received());
    state.configure(Some(policy(0x8123_4567)), 0x8123_4567, 10);
    assert!(!state.allows_movement());
    assert_eq!(verify(&mut state, "cp 9 bad"), Decision::Ignored);
    assert!(!state.received());
    let final_word = (0x8123_4567u32 ^ 7 ^ 0xffff_fffe ^ 2) as i32;
    assert_eq!(
        verify(
            &mut state,
            &format!("cp 11 -128 91 @suffix 7 -2 {final_word}")
        ),
        Decision::Authentic
    );
    assert!(state.allows_movement());
    assert!(state.received());
    assert_eq!(verify(&mut state, "cp 8 malformed"), Decision::Ignored);
    assert!(state.allows_movement());
    state.reset();
    assert!(!state.allows_movement());
    assert!(!state.received());
    assert_eq!(
        verify(
            &mut state,
            &format!("cp 10 -128 91 @ {}", 0x8123_4567u32 as i32)
        ),
        Decision::Authentic
    );
    // Reissued gamestates with an unchanged feed retain checksumFeedServerId.
    state.configure(Some(policy(0x8123_4567)), 0x8123_4567, 20);
    assert_eq!(
        verify(
            &mut state,
            &format!("cp 10 -128 91 @ {}", 0x8123_4567u32 as i32)
        ),
        Decision::Authentic
    );
    state.configure(Some(policy(99)), 99, 21);
    assert_eq!(verify(&mut state, "cp 20 -128 91 @ 99"), Decision::Ignored);
    assert!(!state.allows_movement());
    state.configure(None, 99, 22);
    assert!(state.allows_movement());
}
#[test]
fn malformed_reports_revoke_admission_without_affecting_another_peer() {
    let mut healthy = Admission::default();
    healthy.configure(Some(policy(99)), 99, 10);
    assert_eq!(
        verify(&mut healthy, "cp 10 -128 91 @ 99"),
        Decision::Authentic
    );
    for text in [
        "cp 10",
        "cp 10 @ 91 @ 99",
        "cp 10 -128 @ @ 99",
        "cp 10 -127 91 @ 99",
        "cp 10 -128 90 @ 99",
        "cp 10 -128 91 missing 99",
        "cp 10 -128 91 @ 100",
        "cp 10 -128 91 @ 7 7 97",
        "cp 10 -128 91 @ 8 106",
        "cp 10 -128 91 @ 7 99",
    ] {
        let mut bad = Admission::default();
        bad.configure(Some(policy(99)), 99, 10);
        assert_eq!(verify(&mut bad, "cp 10 -128 91 @ 99"), Decision::Authentic);
        assert_eq!(verify(&mut bad, text), Decision::Rejected, "{text}");
        assert!(bad.received());
        assert!(!bad.allows_movement());
        assert!(healthy.allows_movement());
    }
    let mut missing = Admission::default();
    missing.configure(Some(Policy::new(99, [None, Some(91)], [7])), 99, 10);
    assert_eq!(verify(&mut missing, "cp 10 0 91 @ 99"), Decision::Rejected);
}
#[test]
fn native_server_package_limit_and_client_count_limit_are_enforced() {
    let mut state = Admission::default();
    state.configure(
        Some(Policy::new(
            99,
            [Some(1), Some(2)],
            (0..=PACKAGE_LIMIT as u32).map(|n| n + 100),
        )),
        99,
        10,
    );
    let last = (PACKAGE_LIMIT + 100) as u32;
    let text = format!("cp 10 1 2 @ {last} {}", 99 ^ last ^ 1);
    assert_eq!(verify(&mut state, &text), Decision::Rejected);
    let mut args = vec![b"0".as_slice(); PACKAGE_LIMIT + 6];
    args[..5].copy_from_slice(&[b"cp", b"10", b"1", b"2", b"@"]);
    assert_eq!(
        state.verify(CommandArgs {
            argv: &args,
            raw: &[]
        }),
        Decision::Rejected
    );
}

#[test]
fn reset_of_an_active_peer_resends_once_until_the_new_gamestate_is_published() {
    use qa_network::pure::Movement;
    let mut state = Admission::default();
    state.configure(Some(policy(99)), 99, 10);
    assert_eq!(state.movement(), Movement::Wait);
    assert_eq!(
        verify(&mut state, "cp 10 -128 91 @ 99"),
        Decision::Authentic
    );
    assert_eq!(state.movement(), Movement::EnterWorld);
    state.entered_world();
    assert_eq!(state.movement(), Movement::Allowed);
    state.reset();
    assert_eq!(state.movement(), Movement::ResendGamestate);
    state.configure(Some(policy(99)), 99, 20);
    assert_eq!(state.movement(), Movement::Wait);
    assert_eq!(
        verify(&mut state, "cp 10 -128 91 @ 99"),
        Decision::Authentic
    );
    assert_eq!(state.movement(), Movement::EnterWorld);
    state.entered_world();
    assert_eq!(state.movement(), Movement::Allowed);
}

#[test]
fn disabled_intervals_keep_the_native_feed_epoch_but_new_feeds_replace_it() {
    let mut state = Admission::default();
    state.configure(None, 99, 10);
    state.gamestate(99, 11);
    state.configure(Some(policy(99)), 99, 20);
    assert_eq!(
        verify(&mut state, "cp 10 -128 91 @ 99"),
        Decision::Authentic
    );
    state.configure(None, 99, 21);
    state.configure(Some(policy(99)), 99, 22);
    assert_eq!(
        verify(&mut state, "cp 10 -128 91 @ 99"),
        Decision::Authentic
    );
    state.configure(Some(policy(100)), 100, 23);
    assert_eq!(verify(&mut state, "cp 22 malformed"), Decision::Ignored);
    assert_eq!(
        verify(&mut state, "cp 23 -128 91 @ 100"),
        Decision::Authentic
    );
}

#[test]
fn native_connection_enters_only_after_gamestate_and_a_successful_begin() {
    use qa_network::{pure::Movement, snapshots::Q3_NOT_ACTIVE};
    let mut state = Admission::default();
    assert_eq!(state.movement(), Movement::Allowed);
    assert_eq!(state.snapshot_flags(), 0);
    state.connected();
    assert_eq!(state.movement(), Movement::Wait);
    assert_eq!(state.snapshot_flags(), Q3_NOT_ACTIVE);
    state.configure(None, 99, 1);
    for _ in 0..2 {
        assert_eq!(state.movement(), Movement::EnterWorld);
        assert_eq!(state.snapshot_flags(), Q3_NOT_ACTIVE);
    }
    state.entered_world();
    assert_eq!(state.movement(), Movement::Allowed);
    assert_eq!(state.snapshot_flags(), 0);
    state.gamestate(99, 2);
    assert_eq!(state.snapshot_flags(), Q3_NOT_ACTIVE);
    assert_eq!(state.movement(), Movement::EnterWorld);
}

#[test]
fn live_enforcement_and_unchanged_package_refresh_preserve_native_admission() {
    use qa_network::pure::Movement;
    let mut state = Admission::default();
    state.configure(Some(policy(99)), 99, 10);
    state.set_enabled(false);
    assert_eq!(state.movement(), Movement::EnterWorld);
    state.entered_world();
    assert_eq!(verify(&mut state, "cp malformed"), Decision::Ignored);
    state.set_enabled(true);
    assert_eq!(state.movement(), Movement::ResendGamestate);
    assert_eq!(
        verify(&mut state, "cp 10 -128 91 @ 99"),
        Decision::Authentic
    );
    assert_eq!(state.movement(), Movement::Allowed);
    assert!(!state.refresh_policy(policy(99), 99, 10));
    assert_eq!(state.movement(), Movement::Allowed);
    state.set_enabled(false);
    assert_eq!(verify(&mut state, "cp 10 bad"), Decision::Ignored);
    state.set_enabled(true);
    assert_eq!(state.snapshot_flags(), 0);
    assert!(state.received());
    assert_eq!(state.movement(), Movement::Allowed);
    assert!(state.refresh_policy(policy(100), 100, 20));
    assert!(!state.received());
    assert_eq!(state.movement(), Movement::Wait);
    assert_eq!(verify(&mut state, "cp 10 -128 91 @ 99"), Decision::Ignored);
    assert_eq!(
        verify(&mut state, "cp 20 -128 91 @ 100"),
        Decision::Authentic
    );
    assert_eq!(state.movement(), Movement::EnterWorld);
}

#[test]
fn live_file_policy_compares_native_words_and_ignores_unrelated_systeminfo() {
    use qa_network::pure::files_changed;
    let before = b"\\sv_pure\\1\\fs_game\\MissionPack\\sv_paks\\-1 37\\sv_serverid\\7";
    let after = b"\\SV_PURE\\2\\FS_GAME\\missionpack\\SV_PAKS\\4294967295  037\\sv_serverid\\8\\timescale\\2";
    assert!(!files_changed(before, after).unwrap());
    for changed in [
        b"\\sv_pure\\0\\fs_game\\missionpack\\sv_paks\\-1 37".as_slice(),
        b"\\sv_pure\\1\\fs_game\\newmod\\sv_paks\\-1 37",
        b"\\sv_pure\\1\\fs_game\\missionpack\\sv_paks\\37 -1",
        b"\\sv_pure\\1\\fs_game\\missionpack\\sv_paks\\-1",
        b"\\sv_pure\\1\\fs_game\\missionpack\\sv_paks\\-1 37 99",
    ] {
        assert!(files_changed(before, changed).unwrap());
    }
    let mut packages = qa_network::downloads::PackageChecksums::loaded(after);
    assert_eq!(packages.next().unwrap(), Some(u32::MAX));
    assert_eq!(packages.next().unwrap(), Some(37));
    assert_eq!(packages.next().unwrap(), None);
}
