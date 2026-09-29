//! `GuestServerLogic` end-to-end tests: load synthetic ELF/PE games
//! exporting `vmMain`, invoke commands, and latch failures.

mod common;

use qa_guest::server::{GuestServerLogic, GAME_CLIENT_THINK, GAME_ENTITY_FRAME};

use common::{pe_fixture, test_module};

fn w16(bytes: &mut [u8], offset: usize, value: u16) { bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes()); }
fn w32(bytes: &mut [u8], offset: usize, value: u32) { bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes()); }
fn w64(bytes: &mut [u8], offset: usize, value: u64) { bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes()); }

fn game_elf() -> Vec<u8> {
    let mut bytes = vec![0u8; 4096];
    w32(&mut bytes, 0, 0x464c_457f);
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    w16(&mut bytes, 16, 3);
    w16(&mut bytes, 18, 62);
    w32(&mut bytes, 20, 1);
    w64(&mut bytes, 32, 64);
    w16(&mut bytes, 52, 64);
    w16(&mut bytes, 54, 56);
    w16(&mut bytes, 56, 2);
    // PT_LOAD covers everything with full permissions.
    w32(&mut bytes, 64, 1);
    w32(&mut bytes, 68, 7);
    w64(&mut bytes, 72, 0);
    w64(&mut bytes, 80, 0);
    w64(&mut bytes, 96, 4096);
    w64(&mut bytes, 104, 4096);
    w64(&mut bytes, 112, 4096);
    // PT_DYNAMIC.
    w32(&mut bytes, 120, 2);
    w32(&mut bytes, 124, 7);
    w64(&mut bytes, 128, 0x200);
    w64(&mut bytes, 136, 0x200);
    w64(&mut bytes, 152, 6 * 16);
    w64(&mut bytes, 160, 6 * 16);
    w64(&mut bytes, 168, 8);
    let tags: &[(u64, u64)] = &[(4, 0x480), (5, 0x4a0), (10, 8), (6, 0x400), (11, 24), (0, 0)];
    for (index, (tag, value)) in tags.iter().enumerate() {
        w64(&mut bytes, 0x200 + index * 16, *tag);
        w64(&mut bytes, 0x200 + index * 16 + 8, *value);
    }
    // vmMain symbol: FUNC, global, defined in section 1, value 0x800.
    w32(&mut bytes, 0x418, 1);
    bytes[0x41c] = 18;
    w16(&mut bytes, 0x41e, 1);
    w64(&mut bytes, 0x420, 0x800);
    w64(&mut bytes, 0x428, 6);
    // Hash: 1 bucket, 2 chains.
    w32(&mut bytes, 0x480, 1);
    w32(&mut bytes, 0x484, 2);
    w32(&mut bytes, 0x488, 1);
    w32(&mut bytes, 0x48c, 0);
    w32(&mut bytes, 0x490, 0);
    bytes[0x4a0..0x4a8].copy_from_slice(b"\0vmMain\0");
    // vmMain: mov eax, 42; ret.
    bytes[0x800..0x806].copy_from_slice(&[0xb8, 42, 0, 0, 0, 0xc3]);
    bytes
}

fn game_pe() -> Vec<u8> {
    let mut bytes = pe_fixture(8);
    // Rename the GetGameAPI export to vmMain (fits in the padded name slot).
    let name = b"GetGameAPI\0";
    let start = bytes.windows(name.len()).position(|window| window == name).expect("export name slot");
    bytes[start..start + 7].copy_from_slice(b"vmMain\0");
    // Export target 0x1010 (raw 0x410): mov eax, 7; ret.
    bytes[0x410..0x416].copy_from_slice(&[0xb8, 7, 0, 0, 0, 0xc3]);
    // Entry point 0x1000 (raw 0x400): DllMain must return nonzero.
    bytes[0x400..0x406].copy_from_slice(&[0xb8, 1, 0, 0, 0, 0xc3]);
    bytes
}

#[test]
fn guest_server_logic_loads_elf_and_pe_games_and_invokes_vmmain() {
    let mut logic = GuestServerLogic::new(test_module("server-game")).unwrap();
    assert!(!logic.loaded());
    assert!(logic.vmmain().is_none());
    assert!(logic.failure().is_none());

    logic.load_game(&game_elf()).unwrap();
    assert!(logic.loaded());
    let entry = logic.vmmain().expect("ELF vmMain");
    assert_eq!(entry.offset, 0x1000_0800);
    assert_eq!(logic.call(GAME_CLIENT_THINK, 1, 2, 3).unwrap(), 42);
    assert_eq!(logic.call(GAME_ENTITY_FRAME, 0, 0, 0).unwrap(), 42);
    assert!(logic.failure().is_none());

    // Loading a second game resets all state.
    logic.load_game(&game_pe()).unwrap();
    assert!(logic.loaded());
    assert_eq!(logic.call(GAME_CLIENT_THINK, 9, 9, 9).unwrap(), 7);
    assert!(logic.failure().is_none());

    // Unknown images and missing exports fail loudly.
    assert!(logic.load_game(b"not an image").is_err());
    let mut no_export = game_elf();
    no_export[0x4a1] = b'X';
    assert!(logic.load_game(&no_export).is_err());
    assert_eq!(logic.failure(), Some("game module does not export vmMain"));
    assert!(logic.call(GAME_CLIENT_THINK, 0, 0, 0).is_err());
}
