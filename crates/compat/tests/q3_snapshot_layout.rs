use qa_compat::{
    abi::{
        Q3_ENTITY_STATE_BYTES, Q3_PLAYER_STATE_BYTES, write_q3_entity_state, write_q3_player_state,
    },
    memory::ModuleMemory,
    services::CallError,
};
use qa_network::states::{ENTITY_WORDS, PLAYER_WORDS};

const GUARD: usize = 8;

fn words<const N: usize>() -> [u32; N] {
    std::array::from_fn(|index| 0x8100_0000 | (index as u32 + 1) * 0x0101)
}

fn assert_cells(bytes: &[u8], cells: &[u32]) {
    assert_eq!(bytes.len(), cells.len() * 4);
    for (index, (actual, expected)) in bytes.chunks_exact(4).zip(cells).enumerate() {
        assert_eq!(actual, expected.to_le_bytes(), "native cell {index}");
    }
}

#[test]
fn player_writer_reconstructs_every_public_cell_and_only_null_decode_zeros() {
    assert_eq!(Q3_PLAYER_STATE_BYTES, 468);
    assert_eq!(PLAYER_WORDS, 112);
    let mut w = words::<PLAYER_WORDS>();
    w[26] = i32::MIN as u32;
    w[35] = (i32::MAX - 1) as u32;
    w[43] = 0x7fc1_2345;
    w[44] = (-0.0f32).to_bits();
    w[46] = 1022;
    w[47] = 0xffff_5678;

    // q_shared.h declaration order, rather than msg.c wire order. Only
    // externalEventTime and the final four bookkeeping cells lack netfields.
    let mut expected = vec![
        w[0], w[34], w[3], w[19], w[12], w[1], w[2], w[9], w[4], w[5], w[10], w[8], w[24], w[25],
        w[35], w[26], w[36], w[20], w[11], w[17], w[37], w[14], w[15], w[43], w[44], w[45], w[22],
        w[13], w[16], w[18], w[38], w[39], w[23], w[27], 0, w[40], w[41], w[21], w[7], w[6], w[42],
        w[28], w[29], w[30], w[31], w[32],
    ];
    expected.extend_from_slice(&w[48..64]); // stats
    expected.extend_from_slice(&w[64..80]); // persistant
    expected.extend_from_slice(&w[96..112]); // powerups
    expected.extend_from_slice(&w[80..96]); // ammo
    expected.extend_from_slice(&[w[33], w[47], w[46], 0, 0, 0, 0]);
    assert_eq!(expected.len(), 117);

    for address in [0, 0xffff_f000, 0x1234_5678_0000] {
        let mut storage = [0xa5; Q3_PLAYER_STATE_BYTES + 2 * GUARD];
        {
            let mut memory = ModuleMemory::borrow(address, &mut storage).unwrap();
            write_q3_player_state(&mut memory, address + GUARD as u64, &w).unwrap();
        }
        assert_cells(&storage[GUARD..GUARD + Q3_PLAYER_STATE_BYTES], &expected);
        assert_eq!(&storage[..GUARD], &[0xa5; GUARD]);
        assert_eq!(&storage[GUARD + Q3_PLAYER_STATE_BYTES..], &[0xa5; GUARD]);
    }
}

#[test]
fn entity_writer_preserves_native_number_and_all_trajectory_and_event_cells() {
    assert_eq!(Q3_ENTITY_STATE_BYTES, 208);
    assert_eq!(ENTITY_WORDS, 51);
    let mut w = words::<ENTITY_WORDS>();
    w[0] = i32::MIN as u32;
    w[3] = 0x7fc2_3456;
    w[6] = (-0.0f32).to_bits();
    w[27] = 0xffff_ffff;
    w[31] = 0xffff_5678;
    for address in [0, 0xffff_f000, 0x1234_5678_0000] {
        for number in [0, 42, 1022, 1023] {
            // q_shared.h entityState_t declaration order, including both
            // complete trajectories and its separately encoded number.
            let expected = [
                number, w[11], w[17], w[16], w[0], w[22], w[1], w[2], w[5], w[3], w[4], w[7],
                w[23], w[39], w[40], w[8], w[6], w[41], w[42], w[43], w[44], w[38], w[45], w[24],
                w[25], w[26], w[34], w[35], w[33], w[37], w[21], w[46], w[47], w[10], w[48], w[18],
                w[30], w[15], w[49], w[31], w[29], w[36], w[20], w[50], w[27], w[9], w[13], w[28],
                w[19], w[14], w[12], w[32],
            ];
            let mut storage = [0xa5; Q3_ENTITY_STATE_BYTES + 2 * GUARD];
            {
                let mut memory = ModuleMemory::borrow(address, &mut storage).unwrap();
                write_q3_entity_state(&mut memory, address + GUARD as u64, number, &w).unwrap();
            }
            assert_cells(&storage[GUARD..GUARD + Q3_ENTITY_STATE_BYTES], &expected);
            assert_eq!(&storage[..GUARD], &[0xa5; GUARD]);
            assert_eq!(&storage[GUARD + Q3_ENTITY_STATE_BYTES..], &[0xa5; GUARD]);
        }
    }
}

#[test]
fn player_writer_rejects_short_words_and_invalid_complete_span_without_mutation() {
    let w = words::<PLAYER_WORDS>();
    let mut memory =
        ModuleMemory::load(64, Q3_PLAYER_STATE_BYTES, &[0xa5; Q3_PLAYER_STATE_BYTES]).unwrap();
    assert_eq!(
        write_q3_player_state(&mut memory, 64, &w[..PLAYER_WORDS - 1]),
        Err(CallError::Capacity)
    );
    for base in [63, 65, u64::MAX - 16] {
        assert_eq!(
            write_q3_player_state(&mut memory, base, &w),
            Err(CallError::Memory)
        );
    }
    assert_eq!(
        memory.read(64, Q3_PLAYER_STATE_BYTES).unwrap(),
        &[0xa5; Q3_PLAYER_STATE_BYTES]
    );
    write_q3_player_state(&mut memory, 64, &w).unwrap();
    assert_eq!(memory.read_word(64 + 448).unwrap() as u32, w[46]);
}

#[test]
fn entity_writer_rejects_short_words_and_invalid_complete_span_without_mutation() {
    let w = words::<ENTITY_WORDS>();
    let mut memory =
        ModuleMemory::load(64, Q3_ENTITY_STATE_BYTES, &[0xa5; Q3_ENTITY_STATE_BYTES]).unwrap();
    assert_eq!(
        write_q3_entity_state(&mut memory, 64, 0, &w[..ENTITY_WORDS - 1]),
        Err(CallError::Capacity)
    );
    for base in [63, 65, u64::MAX - 16] {
        assert_eq!(
            write_q3_entity_state(&mut memory, base, 0, &w),
            Err(CallError::Memory)
        );
    }
    assert_eq!(
        memory.read(64, Q3_ENTITY_STATE_BYTES).unwrap(),
        &[0xa5; Q3_ENTITY_STATE_BYTES]
    );
    write_q3_entity_state(&mut memory, 64, 0, &w).unwrap();
    assert_eq!(memory.read_word(64).unwrap(), 0);
}
