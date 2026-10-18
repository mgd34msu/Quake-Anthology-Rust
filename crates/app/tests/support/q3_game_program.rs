//! One synthetic public-Q3 Game fixture shared by lifecycle and connection tests.
use qa_formats::program::qvm::Opcode as Op;

pub const COUNTERS: i32 = 128;
pub const CONNECT_CLIENT: i32 = 176;
pub const FIRST_TIME: i32 = 180;
pub const IS_BOT: i32 = 184;
pub const THINK_CLIENT: i32 = 188;
pub const COMMAND_ARGC: i32 = 192;
pub const COMMAND_ARGV: i32 = 256;
pub const USERINFO: i32 = 384;
pub const GAME_USERINFO: i32 = 768;
pub const CHANGED_USERINFO: &[u8] = b"\\name\\GameName\\skin\\game/blue\0";
pub const COMMAND: i32 = 512;
pub const ENTITIES: i32 = 4096;
// Public sharedEntity_t is 516 bytes, distinct from the 272-byte vmCvar_t.
pub const ENTITY_STRIDE: i32 = 608;
pub const PLAYERS: i32 = 16384;
pub const PLAYER_STRIDE: i32 = 512;
pub const PLAYER_ORIGIN: i32 = PLAYERS + 20;

pub type Code = Vec<(Op, i32)>;
fn word(code: &mut Code, address: i32, value: i32) {
    code.extend([(Op::Const, address), (Op::Const, value), (Op::Store4, 0)]);
}
fn parameter(code: &mut Code, address: i32, index: i32) {
    code.extend([
        (Op::Const, address),
        (Op::Local, 76 + index * 4),
        (Op::Load4, 0),
        (Op::Store4, 0),
    ]);
}
fn increment(code: &mut Code, address: i32) {
    code.extend([
        (Op::Const, address),
        (Op::Const, address),
        (Op::Load4, 0),
        (Op::Const, 1),
        (Op::Add, 0),
        (Op::Store4, 0),
    ]);
}
pub fn import(code: &mut Code, number: i32, arguments: &[i32], client: bool) {
    for (index, &value) in arguments.iter().enumerate() {
        if client && index == 0 {
            code.extend([(Op::Local, 76), (Op::Load4, 0)]);
        } else {
            code.push((Op::Const, value));
        }
        code.push((Op::Arg, 8 + index as i32 * 4));
    }
    code.extend([(Op::Const, -number - 1), (Op::Call, 0)]);
}
pub fn finish(code: &mut Code, value: i32) {
    code.extend([(Op::Const, value), (Op::Leave, 64)]);
}

pub fn code(denial: i32, fail_begin: bool, userinfo_mode: i32) -> (Code, Vec<u8>) {
    let mut code = vec![(Op::Enter, 64)];
    let mut branches = Vec::new();
    for export in 0..=9 {
        code.extend([
            (Op::Local, 72),
            (Op::Load4, 0),
            (Op::Const, export),
            (Op::Eq, 0),
        ]);
        branches.push(code.len() - 1);
    }
    finish(&mut code, 0);
    for export in 0..=9 {
        code[branches[export as usize]].1 = code.len() as i32;
        if export != 9 {
            increment(&mut code, COUNTERS + export * 4);
        }
        match export {
            0 => {
                import(
                    &mut code,
                    15,
                    &[ENTITIES, 2, ENTITY_STRIDE, PLAYERS, PLAYER_STRIDE],
                    false,
                );
                code.push((Op::Pop, 0));
            }
            2 => {
                parameter(&mut code, CONNECT_CLIENT, 0);
                parameter(&mut code, FIRST_TIME, 1);
                parameter(&mut code, IS_BOT, 2);
                import(&mut code, 20, &[0, USERINFO, 64], true);
                code.push((Op::Pop, 0));
                finish(&mut code, denial);
                continue;
            }
            3 => {
                if fail_begin {
                    code.extend([(Op::Const, 1), (Op::Const, 0), (Op::DivI, 0), (Op::Pop, 0)]);
                }
                // A module-owned noclip pose makes an accidental host move observable.
                word(&mut code, PLAYERS + 4, 1); // PM_NOCLIP
                word(&mut code, PLAYER_ORIGIN, 10.0f32.to_bits() as i32);
                word(&mut code, ENTITIES + 488, 10.0f32.to_bits() as i32);
                word(&mut code, PLAYERS + 32, 64.0f32.to_bits() as i32);
                word(&mut code, PLAYERS + 48, 800);
                word(&mut code, PLAYERS + 52, 320);
                word(&mut code, PLAYERS + 56, 65536); // native signed32 delta angle
                word(&mut code, PLAYERS + 152, 45.0f32.to_bits() as i32);
                word(&mut code, PLAYERS + 184, 125); // STAT_HEALTH
            }
            4 => {
                import(&mut code, 20, &[0, USERINFO, 64], true);
                code.push((Op::Pop, 0));
                if userinfo_mode != 0 {
                    import(&mut code, 21, &[0, GAME_USERINFO], true);
                    code.push((Op::Pop, 0));
                }
                if userinfo_mode < 0 {
                    code.extend([(Op::Const, 1), (Op::Const, 0), (Op::DivI, 0), (Op::Pop, 0)]);
                }
            }
            6 => {
                code.push((Op::Const, COMMAND_ARGC));
                import(&mut code, 8, &[], false);
                code.push((Op::Store4, 0));
                import(&mut code, 9, &[1, COMMAND_ARGV, 64], false);
                code.push((Op::Pop, 0));
            }
            7 => {
                parameter(&mut code, THINK_CLIENT, 0);
                import(&mut code, 36, &[0, COMMAND], true);
                code.push((Op::Pop, 0));
                code.extend([
                    (Op::Const, PLAYERS),
                    (Op::Const, COMMAND),
                    (Op::Load4, 0),
                    (Op::Store4, 0),
                    (Op::Const, PLAYER_ORIGIN),
                    (Op::Const, PLAYER_ORIGIN),
                    (Op::Load4, 0),
                    (Op::Const, 1.0f32.to_bits() as i32),
                    (Op::AddF, 0),
                    (Op::Store4, 0),
                    // Collision pose is module-owned independently of playerState.
                    (Op::Const, ENTITIES + 488),
                    (Op::Const, PLAYER_ORIGIN),
                    (Op::Load4, 0),
                    (Op::Store4, 0),
                ]);
            }
            9 => {
                code.extend([
                    (Op::Local, 76),
                    (Op::Load4, 0),
                    (Op::Load4, 0),
                    (Op::Leave, 64),
                ]);
                continue;
            }
            _ => {}
        }
        finish(&mut code, 0);
    }
    let mut data = vec![0; PLAYERS as usize + 2 * PLAYER_STRIDE as usize];
    data[..14].copy_from_slice(b"masked denial\0");
    data[GAME_USERINFO as usize..GAME_USERINFO as usize + CHANGED_USERINFO.len()]
        .copy_from_slice(CHANGED_USERINFO);
    for index in 0..2 {
        let at = ENTITIES as usize + index * ENTITY_STRIDE as usize;
        data[at..at + 4].copy_from_slice(&(index as i32).to_le_bytes());
        data[at + 512..at + 516].copy_from_slice(&1023i32.to_le_bytes());
        for (offset, values) in [(436, [-16.0f32, -16.0, -24.0]), (448, [16.0, 16.0, 32.0])] {
            for (axis, value) in values.into_iter().enumerate() {
                data[at + offset + axis * 4..at + offset + axis * 4 + 4]
                    .copy_from_slice(&value.to_bits().to_le_bytes());
            }
        }
    }
    (code, data)
}
