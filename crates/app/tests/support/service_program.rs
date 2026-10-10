use qa_compat::qvm::Vm;
use qa_formats::program::qvm::{Image, Opcode};

pub fn qvm() -> Vm {
    use Opcode::*;
    let operations: [(Opcode, i32); 25] = [
        (Enter, 64),
        (Const, 16),
        (Arg, 8),
        (Const, 32),
        (Arg, 12),
        (Const, -6),
        (Call, 0),
        (Pop, 0),
        (Const, 48),
        (Arg, 8),
        (Const, -1),
        (Call, 0),
        (Pop, 0),
        (Const, 2),
        (Arg, 8),
        (Const, 80),
        (Arg, 12),
        (Const, -15),
        (Call, 0),
        (Pop, 0),
        (Const, 16),
        (Arg, 8),
        (Const, -7),
        (Call, 0),
        (Leave, 64),
    ];
    let mut code = Vec::new();
    for (op, arg) in operations {
        code.push(op as u8);
        match op.operand_bytes() {
            1 => code.push(arg as u8),
            4 => code.extend(arg.to_le_bytes()),
            _ => {}
        }
    }
    code.resize((code.len() + 3) & !3, 0);
    let mut bytes = Vec::new();
    for word in [
        0x12721444,
        operations.len() as i32,
        32,
        code.len() as i32,
        32 + code.len() as i32,
        128,
        0,
        65536 - 128,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(code);
    let data = bytes.len();
    bytes.resize(data + 128, 0);
    for (offset, text) in [
        (16, &b"fov\0"[..]),
        (32, b"105\0"),
        (48, b"module print\n\0"),
        (80, b"sensitivity 7\n\0"),
    ] {
        bytes[data + offset..data + offset + text.len()].copy_from_slice(text);
    }
    Vm::load(Image::parse(&bytes).unwrap()).unwrap()
}
