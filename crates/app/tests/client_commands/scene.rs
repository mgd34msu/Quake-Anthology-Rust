#[path = "live_formats.rs"]
mod live_formats;
#[path = "loading_screen.rs"]
mod loading_screen;
#[path = "../../../render/tests/support/world_surfaces.rs"]
mod mark_world;
#[path = "skin_download.rs"]
mod skin_download;
use super::*;
pub(super) use live_formats::live_formats;
pub(super) use loading_screen::loading_screen;
use qa_app::model_assets::{ModelAssets, ModelRequest};
use qa_compat::services::ModelLookup;
use qa_render::material::world_load::{
    LoadedWorld, LoadedWorldPresentation, WorldPresentationProfile,
};
use qa_render::scene::Command;
pub(super) use skin_download::downloaded_skins;

fn admit_assets(host: &mut FrameHost) -> qa_render::cpu::CpuBackend {
    let backend = qa_render::cpu::CpuBackend::load_with_assets(
        320,
        200,
        host.runtime.model_assets.as_ref().unwrap().assets(),
    )
    .unwrap();
    host.publish_model_admission(backend.admitted_assets().unwrap())
        .unwrap();
    backend
}

const SHADER_HANDLE: u32 = 864;
const LOD_SCALE_NAME: u32 = 208;
const LOD_BIAS_NAME: u32 = 240;
const CURVE_NAME: u32 = 976;
const CURVE_FIRST: u32 = 992;
const CURVE_SECOND: u32 = 1000;
const NOVIS_NAME: u32 = 1024;
const NOCULL_NAME: u32 = 1040;
const WORLD_NAME: u32 = 1056;
const ENTITIES_NAME: u32 = 1072;
const NEAR_NAME: u32 = 1096;
const POLICY_FIRST: u32 = 1112;
const POLICY_SECOND: u32 = 1120;
const VIEW_CALLS: &[(u8, &[u32])] = &[
    (5, &[CURVE_NAME, CURVE_FIRST]),
    (5, &[LOD_SCALE_NAME, CURVE_FIRST]),
    (5, &[LOD_BIAS_NAME, CURVE_SECOND]),
    (5, &[NOVIS_NAME, POLICY_FIRST]),
    (5, &[NOCULL_NAME, POLICY_FIRST]),
    (5, &[WORLD_NAME, POLICY_FIRST]),
    (5, &[ENTITIES_NAME, POLICY_FIRST]),
    (5, &[NEAR_NAME, POLICY_FIRST]),
    (44, &[256]),
    (5, &[CURVE_NAME, CURVE_SECOND]),
    (5, &[LOD_SCALE_NAME, CURVE_SECOND]),
    (5, &[LOD_BIAS_NAME, CURVE_FIRST]),
    (5, &[NOVIS_NAME, POLICY_SECOND]),
    (5, &[NOCULL_NAME, POLICY_SECOND]),
    (5, &[WORLD_NAME, POLICY_SECOND]),
    (5, &[ENTITIES_NAME, POLICY_SECOND]),
    (5, &[NEAR_NAME, POLICY_SECOND]),
    (44, &[256]),
];
const CALLS: &[(u8, &[u32])] = &[
    (39, &[16]),
    (45, &[32]),
    (87, &[0, 3, 64, 2]),
    (44, &[256]),
    (
        46,
        &[
            (-10.0f32).to_bits(),
            0,
            100.0f32.to_bits(),
            30.0f32.to_bits(),
            0,
            0,
            1.0f32.to_bits(),
            1.0f32.to_bits(),
            0,
        ],
    ),
    (40, &[]),
    (36, &[224]),
    (87, &[0, 3, 65528, 2]),
    (45, &[0]),
    (87, &[0, 3, u32::MAX, 2]),
];

const MARK_CALL: (u8, &[u32]) = (27, &[4, 640, 688, 16, 32, 8, 940]);
fn scene_call(command: usize) -> (u8, &'static [u32]) {
    if command == 20 {
        MARK_CALL
    } else {
        CALLS[command]
    }
}

fn data() -> [u8; 1536] {
    let mut data = [0; 1536];
    data[16..27].copy_from_slice(b"ui/pic.tga\0");
    data[224..239].copy_from_slice(b"maps/probe.bsp\0");
    for (index, value) in [0.5f32, 0.25, 1., 0.75].into_iter().enumerate() {
        data[32 + index * 4..36 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    for index in 0..6 {
        let first = 64 + index * 24;
        for (axis, value) in [8.0, index as f32, (index % 3) as f32]
            .into_iter()
            .enumerate()
        {
            data[first + axis * 4..first + axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[first + 20..first + 24].copy_from_slice(&[index as u8, 20, 30, 255]);
    }
    for (at, value) in [(8, 160i32), (12, 200), (72, -12345), (76, 1)] {
        data[256 + at..260 + at].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [(16, 95.0f32), (20, 75.), (36, 1.), (52, 1.), (68, 1.)] {
        data[256 + at..260 + at].copy_from_slice(&value.to_le_bytes());
    }
    data[336..368].fill(0xa5);
    for row in 0..8 {
        data[368 + row * 32 + 1] = 0xe0 + row as u8;
    }
    for (index, point) in [[0f32, -2., -2.], [0., 2., -2.], [0., 2., 2.], [0., -2., 2.]]
        .into_iter()
        .enumerate()
    {
        for (axis, coordinate) in point.into_iter().enumerate() {
            let at = 640 + index * 12 + axis * 4;
            data[at..at + 4].copy_from_slice(&coordinate.to_le_bytes());
        }
    }
    data[688..692].copy_from_slice(&2f32.to_le_bytes());
    data[880..896].copy_from_slice(b"models/test.md3\0");
    data[900..917].copy_from_slice(b"models/test.skin\0");
    data[920..923].copy_from_slice(b"*1\0");
    for (offset, value) in [
        (28, 1.0f32),
        (44, 1.),
        (60, 1.),
        (68, 8.),
        (100, 0.25),
        (120, 0.125),
        (124, 0.5),
        (128, 1.25),
        (132, 2.),
        (136, 45.),
    ] {
        data[700 + offset..704 + offset].copy_from_slice(&value.to_le_bytes());
    }
    data[780..784].copy_from_slice(&1i32.to_le_bytes());
    data[816..820].copy_from_slice(&[201, 150, 90, 255]);
    for (offset, text) in [
        (CURVE_NAME, b"r_lodCurveError\0".as_slice()),
        (LOD_SCALE_NAME, b"r_lodscale\0"),
        (LOD_BIAS_NAME, b"r_lodbias\0"),
        (CURVE_FIRST, b"12.5\0"),
        (CURVE_SECOND, b"-2\0"),
        (NOVIS_NAME, b"r_novis\0"),
        (NOCULL_NAME, b"r_nocull\0"),
        (WORLD_NAME, b"r_drawworld\0"),
        (ENTITIES_NAME, b"r_drawentities\0"),
        (NEAR_NAME, b"r_znear\0"),
        (POLICY_FIRST, b"0.5\0"),
        (POLICY_SECOND, b"2\0"),
    ] {
        data[offset as usize..offset as usize + text.len()].copy_from_slice(text);
    }
    data
}

fn pointer_argument(command: usize, index: usize) -> bool {
    matches!(
        (command, index),
        (0 | 1 | 3 | 6 | 8 | 12 | 13, 0) | (2 | 7 | 9, 2) | (20, 1 | 2 | 4 | 6)
    )
}
fn shader_argument(command: usize, index: usize) -> bool {
    matches!((command, index), (2 | 7, 0) | (4, 8))
}
fn ordinal(ui: bool, value: u8) -> u8 {
    if !ui {
        return value;
    }
    match value {
        5 => 3,
        39 => 20,
        45 => 26,
        87 => 23,
        44 => 25,
        46 => 27,
        40 => 21,
        37 => 18,
        38 => 19,
        41 => 22,
        79 => 80,
        other => other,
    }
}
fn qvm_scene(ui: bool) -> Program {
    qvm_scene_data(ui, &data())
}
fn qvm_scene_data(ui: bool, initial: &[u8; 1536]) -> Program {
    use Opcode::*;
    let mut operations = vec![(Enter, 96)];
    let mut branches = vec![];
    for command in 0..34 {
        operations.extend([(Local, 104), (Load4, 0), (Const, command), (Eq, 0)]);
        branches.push(operations.len() - 1);
    }
    operations.extend([(Const, 0), (Leave, 96)]);
    for (command, branch) in branches.into_iter().enumerate() {
        operations[branch].1 = operations.len() as i32;
        if command == 31 {
            for value in 1..=2 {
                operations.extend([
                    (Const, 1460),
                    (Const, value),
                    (Store4, 0),
                    (Const, if ui { -29 } else { -18 }),
                    (Call, 0),
                    (Pop, 0),
                ]);
            }
            operations.extend([(Const, 1460), (Const, 3), (Store4, 0), (Const, 73)]);
        } else if command == 32 || ui && command == 9 && initial[1472] != 0 {
            operations.extend([
                (Const, 1456),
                (Const, 1456),
                (Load4, 0),
                (Const, 1),
                (Add, 0),
                (Store4, 0),
                (Const, 1464),
                (Local, 108),
                (Load4, 0),
                (Store4, 0),
                (Const, if ui { -29 } else { -18 }),
                (Call, 0),
                (Pop, 0),
                (Const, SHADER_HANDLE as i32),
                (Const, 16),
                (Arg, 8),
                (Const, -i32::from(ordinal(ui, 39)) - 1),
                (Call, 0),
                (Store4, 0),
            ]);
            let (number, words) = CALLS[4];
            for (index, &word) in words.iter().enumerate() {
                if index == 8 {
                    operations.extend([(Const, SHADER_HANDLE as i32), (Load4, 0)]);
                } else {
                    operations.push((Const, word as i32));
                }
                operations.push((Arg, 8 + index as i32 * 4));
            }
            operations.extend([
                (Const, -i32::from(ordinal(ui, number)) - 1),
                (Call, 0),
                (Pop, 0),
            ]);
            operations.extend([(Const, 1468), (Load4, 0), (Const, 0), (Eq, 0)]);
            let good = operations.len() - 1;
            operations.extend([
                (Const, 16),
                (Arg, 8),
                (Const, if ui { -1 } else { -2 }),
                (Call, 0),
                (Pop, 0),
            ]);
            operations[good].1 = operations.len() as i32;
            operations.push((Const, 0));
        } else if command == 33 {
            operations.extend([(Local, 108), (Load4, 0), (Load4, 0)]);
        } else if command == 30 {
            operations.extend([
                (Const, 208),
                (Arg, 8),
                (Local, 108),
                (Load4, 0),
                (Arg, 12),
                (Const, -i32::from(ordinal(ui, 5)) - 1),
                (Call, 0),
                (Pop, 0),
                (Local, 112),
                (Load4, 0),
                (Arg, 8),
                (Const, -i32::from(ordinal(ui, 39)) - 1),
                (Call, 0),
            ]);
        } else if command == 29 {
            for &(number, words) in VIEW_CALLS {
                for (index, &word) in words.iter().enumerate() {
                    operations.extend([(Const, word as i32), (Arg, 8 + index as i32 * 4)]);
                }
                operations.extend([
                    (Const, -i32::from(ordinal(ui, number)) - 1),
                    (Call, 0),
                    (Pop, 0),
                ]);
            }
            operations.push((Const, 0));
        } else if command == 26 {
            // The reached malformed-string witness names the final masked byte.
            operations.extend([(Const, 65535), (Const, 255), (Store1, 0)]);
            for index in 0..3 {
                operations.extend([(Local, 108 + index * 4), (Load4, 0), (Arg, 8 + index * 4)]);
            }
            operations.extend([(Const, -i32::from(ordinal(ui, 79)) - 1), (Call, 0)]);
        } else if command == 27 {
            operations.extend([
                (Local, 108),
                (Load4, 0),
                (Arg, 8),
                (Const, -i32::from(ordinal(ui, 39)) - 1),
                (Call, 0),
            ]);
        } else if command == 28 {
            operations.extend([
                (Const, SHADER_HANDLE as i32),
                (Local, 108),
                (Load4, 0),
                (Store4, 0),
                (Const, 0),
            ]);
        } else if matches!(command, 12 | 13 | 16 | 19) {
            let (number, address) = match command {
                12 => (37, 880),
                16 => (38, 900),
                19 => (37, 920),
                _ => (41, 700),
            };
            operations.extend([
                (Const, address),
                (Arg, 8),
                (Const, -i32::from(ordinal(ui, number)) - 1),
                (Call, 0),
            ]);
        } else if matches!(command, 14 | 15 | 17 | 18) {
            operations.extend([
                (
                    Const,
                    match command {
                        14 => 708,
                        17 => 808,
                        18 => 812,
                        _ => 700,
                    },
                ),
                (Local, 108),
                (Load4, 0),
                (Store4, 0),
                (Const, 0),
            ]);
        } else if command >= 21 {
            operations.extend([
                (
                    Const,
                    match command {
                        21 => 940,
                        22 => 944,
                        23 => 32,
                        24 => 36,
                        _ => 40,
                    },
                ),
                (Load4, 0),
            ]);
        } else if command >= CALLS.len() && command != 20 {
            for (at, word) in if command == 10 {
                [(64, 900.0f32.to_bits()), (328, 777)]
            } else {
                [(332, 0), (332, 0)]
            } {
                operations.extend([(Const, at), (Const, word as i32), (Store4, 0)]);
            }
            operations.push((Const, 0));
        } else {
            let (number, words) = scene_call(command);
            let ordinal = ordinal(ui, number);
            if command == 0 {
                operations.push((Const, SHADER_HANDLE as i32));
            }
            for (index, &word) in words.iter().enumerate() {
                if shader_argument(command, index) {
                    operations.extend([(Const, SHADER_HANDLE as i32), (Load4, 0)]);
                } else {
                    operations.push((Const, word as i32));
                }
                operations.push((Arg, 8 + index as i32 * 4));
            }
            operations.extend([(Const, -i32::from(ordinal) - 1), (Call, 0)]);
            if command == 0 {
                operations.extend([
                    (Store4, 0),
                    (Const, 812),
                    (Const, SHADER_HANDLE as i32),
                    (Load4, 0),
                    (Store4, 0),
                    (Const, SHADER_HANDLE as i32),
                    (Load4, 0),
                ]);
            }
        }
        operations.push((Leave, 96));
    }
    Program::Qvm {
        vm: Box::new(service_program::program(&operations, initial)),
        imports: if ui {
            &qa_compat::abi::Q3_UI
        } else {
            &Q3_CLIENT
        },
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_scene(ui: bool) -> Program {
    native_scene_data(ui, &data())
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_scene_data(ui: bool, initial: &[u8; 1536]) -> Program {
    use native_image::*;
    use qa_compat::native::{NamedExport, Vm};
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;
    let mut file = pe_export_fixture(64);
    put(&mut file, 392 + 8, 8192, 4);
    put(&mut file, 392 + 16, 8192, 4);
    put(&mut file, 208, 12288, 4);
    file.resize(8704, 0);
    pe_rva(&mut file, 0x1140, 0x1800, 4);
    pe_text(&mut file, 0x1800, &[0xc3]);
    let image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let base = image.base;
    let mut vm = Vm::map_image(
        image,
        &(0..34)
            .map(|command| NamedExport {
                name: b"GetGameAPI",
                command: Some(command),
                parameters: &[NativeScalar::Word; 13],
                result: NativeScalar::Word,
            })
            .collect::<Vec<_>>(),
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    let callback = vm.import_callback();
    let source = base + 0x2800;
    let mut code = vec![];
    for command in 0..34usize {
        code.extend([0x83, 0xf9, command as u8, 0x0f, 0x85]);
        let branch = code.len();
        code.extend([0; 4]);
        code.extend([0x48, 0x83, 0xec, 0x58]);
        if command == 31 {
            for value in 1..=2u32 {
                code.extend([0x48, 0xb8]);
                code.extend((source + 1460).to_le_bytes());
                code.extend([0xc7, 0x00]);
                code.extend(value.to_le_bytes());
                code.extend([0xb9, if ui { 28 } else { 17 }, 0, 0, 0, 0x48, 0xb8]);
                code.extend(callback.to_le_bytes());
                code.extend([0xff, 0xd0]);
            }
            code.extend([0x48, 0xb8]);
            code.extend((source + 1460).to_le_bytes());
            code.extend([0xc7, 0x00, 3, 0, 0, 0, 0xb8, 73, 0, 0, 0]);
        } else if command == 32 || ui && command == 9 && initial[1472] != 0 {
            code.extend([0x48, 0xb8]);
            code.extend((source + 1456).to_le_bytes());
            code.extend([0xff, 0x00, 0x89, 0x50, 8]); // count++; time = edx
            code.extend([0xb9, if ui { 28 } else { 17 }, 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0, 0x48, 0xba]);
            code.extend((source + 16).to_le_bytes());
            code.extend([0xb9, ordinal(ui, 39), 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0, 0x48, 0xb9]);
            code.extend((source + u64::from(SHADER_HANDLE)).to_le_bytes());
            code.extend([0x89, 0x01]);
            let (number, words) = CALLS[4];
            for (index, &word) in words.iter().enumerate() {
                code.extend(if index < 3 {
                    [[0x48, 0xba], [0x49, 0xb8], [0x49, 0xb9]][index]
                } else {
                    [0x48, 0xb8]
                });
                code.extend(u64::from(word).to_le_bytes());
                if index >= 3 {
                    if index == 8 {
                        code.extend([0x48, 0xb8]);
                        code.extend((source + u64::from(SHADER_HANDLE)).to_le_bytes());
                        code.extend([0x8b, 0x00]);
                    }
                    code.extend([0x48, 0x89, 0x44, 0x24, (0x20 + (index - 3) * 8) as u8]);
                }
            }
            code.extend([0xb9, ordinal(ui, number), 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0, 0x48, 0xb8]);
            code.extend((source + 1468).to_le_bytes());
            code.extend([0x83, 0x38, 0, 0x74, 27, 0x48, 0xba]);
            code.extend((source + 16).to_le_bytes());
            code.extend([0xb9, if ui { 0 } else { 1 }, 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0, 0x31, 0xc0]);
        } else if command == 33 {
            code.extend([0x48, 0xb8]);
            code.extend(source.to_le_bytes());
            code.extend([0x8b, 0x04, 0x10]);
        } else if command == 30 {
            // Preserve the shader offset across Cvar_Set, then register it
            // without leaving this owned-child callback.
            code.extend([0x4c, 0x89, 0x44, 0x24, 0x40, 0x49, 0x89, 0xd0, 0x48, 0xb8]);
            code.extend(source.to_le_bytes());
            code.extend([0x49, 0x01, 0xc0, 0x48, 0xba]);
            code.extend((source + 208).to_le_bytes());
            code.extend([0xb9, ordinal(ui, 5), 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0, 0x48, 0x8b, 0x54, 0x24, 0x40, 0x48, 0xb8]);
            code.extend(source.to_le_bytes());
            code.extend([0x48, 0x01, 0xc2, 0xb9, ordinal(ui, 39), 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0]);
        } else if command == 29 {
            for &(number, words) in VIEW_CALLS {
                for (index, &word) in words.iter().enumerate() {
                    code.extend(if index == 0 {
                        [0x48, 0xba]
                    } else {
                        [0x49, 0xb8]
                    });
                    code.extend((source + u64::from(word)).to_le_bytes());
                }
                code.extend([0xb9, ordinal(ui, number), 0, 0, 0, 0x48, 0xb8]);
                code.extend(callback.to_le_bytes());
                code.extend([0xff, 0xd0]);
            }
        } else if command == 26 || command == 27 {
            let registers = if command == 26 { 3 } else { 1 };
            for index in 0..registers {
                code.extend(match index {
                    0 => [0x48, 0x85, 0xd2],
                    1 => [0x4d, 0x85, 0xc0],
                    _ => [0x4d, 0x85, 0xc9],
                });
                code.extend([0x74, 13, 0x48, 0xb8]);
                code.extend(source.to_le_bytes());
                code.extend(match index {
                    0 => [0x48, 0x01, 0xc2],
                    1 => [0x49, 0x01, 0xc0],
                    _ => [0x49, 0x01, 0xc1],
                });
            }
            code.extend([
                0xb9,
                ordinal(ui, if command == 26 { 79 } else { 39 }),
                0,
                0,
                0,
                0x48,
                0xb8,
            ]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0]);
        } else if command == 28 {
            code.extend([0x48, 0xb8]);
            code.extend((source + u64::from(SHADER_HANDLE)).to_le_bytes());
            code.extend([0x89, 0x10, 0x31, 0xc0]);
        } else if matches!(command, 12 | 13 | 16 | 19) {
            let (number, address) = match command {
                12 => (37, 880),
                16 => (38, 900),
                19 => (37, 920),
                _ => (41, 700),
            };
            code.extend([0x48, 0xba]);
            code.extend((source + address).to_le_bytes());
            code.extend([0xb9, ordinal(ui, number), 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0]);
        } else if matches!(command, 14 | 15 | 17 | 18) {
            code.extend([0x48, 0xb8]);
            code.extend(
                (source
                    + match command {
                        14 => 708,
                        17 => 808,
                        18 => 812,
                        _ => 700,
                    })
                .to_le_bytes(),
            );
            code.extend([0x89, 0x10, 0x31, 0xc0]);
        } else if command >= 21 {
            code.extend([0x48, 0xb8]);
            code.extend(
                (source
                    + match command {
                        21 => 940,
                        22 => 944,
                        23 => 32,
                        24 => 36,
                        _ => 40,
                    })
                .to_le_bytes(),
            );
            code.extend([0x8b, 0x00]);
        } else if command >= CALLS.len() && command != 20 {
            for (at, word) in if command == 10 {
                [(64, 900.0f32.to_bits()), (328, 777)]
            } else {
                [(332, 0), (332, 0)]
            } {
                code.extend([0x48, 0xb8]);
                code.extend((source + at).to_le_bytes());
                code.extend([0xc7, 0x00]);
                code.extend(word.to_le_bytes());
            }
            code.extend([0x31, 0xc0]);
        } else {
            let (number, arguments) = scene_call(command);
            let ordinal = ordinal(ui, number);
            for (index, &argument) in arguments.iter().enumerate() {
                if shader_argument(command, index) {
                    code.extend([0x48, 0xb8]);
                    code.extend((source + u64::from(SHADER_HANDLE)).to_le_bytes());
                    if index < 3 {
                        code.extend(match index {
                            0 => [0x8b, 0x10].as_slice(),
                            1 => [0x44, 0x8b, 0x00].as_slice(),
                            _ => [0x44, 0x8b, 0x08].as_slice(),
                        });
                    } else {
                        code.extend([0x8b, 0x00]);
                        code.extend([0x48, 0x89, 0x44, 0x24, (0x20 + (index - 3) * 8) as u8]);
                    }
                    continue;
                }
                let value = if pointer_argument(command, index) && argument != 0 {
                    source + u64::from(argument)
                } else {
                    u64::from(argument)
                };
                if index < 3 {
                    code.extend([[0x48, 0xba], [0x49, 0xb8], [0x49, 0xb9]][index]);
                    code.extend(value.to_le_bytes());
                } else {
                    code.extend([0x48, 0xb8]);
                    code.extend(value.to_le_bytes());
                    code.extend([0x48, 0x89, 0x44, 0x24, (0x20 + (index - 3) * 8) as u8]);
                }
            }
            code.extend([0xb9, ordinal, 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0]);
            if command == 0 {
                for address in [u64::from(SHADER_HANDLE), 812] {
                    code.extend([0x48, 0xb9]);
                    code.extend((source + address).to_le_bytes());
                    code.extend([0x89, 0x01]);
                }
            }
        }
        code.extend([0x48, 0x83, 0xc4, 0x58, 0xc3]);
        let end = code.len();
        code[branch..branch + 4].copy_from_slice(&((end - branch - 4) as i32).to_le_bytes());
    }
    code.extend([0x31, 0xc0, 0xc3]);
    assert!(code.len() < 0x1000);
    vm.process.memory_mut().unwrap()[0x1800..0x1800 + code.len()].copy_from_slice(&code);
    vm.process.memory_mut().unwrap()[0x2800..0x2800 + initial.len()].copy_from_slice(initial);
    Program::Native {
        vm: Box::new(vm),
        imports: if ui {
            &qa_compat::abi::Q3_UI
        } else {
            &Q3_CLIENT
        },
    }
}

pub(super) fn submissions(native: bool, ui: bool) {
    if !ui {
        mark_fragments(native);
        sun_contexts(native);
    }
    let mut fixture = model_support::Fixture::new(&[(
        "ui/pic.tga",
        model_support::tga(2, 2, &[[80, 30, 10, 255]; 4]),
    )]);
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut assets = qa_render::Assets::load().unwrap();
    let policy_rules = [
        RuleSetId::Quake3,
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
    ];
    let variants: [&[qa_render::world::SurfaceMaterial]; 5] = [&[]; 5];
    let world = model_support::world_variants(&mut assets, -1., &variants);
    let presentation = qa_render::world::WorldPresentationId(0);
    let policy_presentations = assets.world(world).unwrap().presentations().to_vec();
    let loaded = LoadedWorld {
        world,
        presentations: policy_rules
            .into_iter()
            .zip(policy_presentations.iter().copied())
            .map(|(rules, id)| LoadedWorldPresentation {
                id,
                profile: WorldPresentationProfile::native(rules, None),
                lighting: Default::default(),
                compile_policy: Default::default(),
                defaults: Default::default(),
            })
            .collect(),
        diagnostics: Box::default(),
    };
    assets
        .register_world_presentation_metadata(world, &loaded.presentations, None)
        .unwrap();
    let program = || {
        if native {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            {
                return native_scene(ui);
            }
        }
        qvm_scene(ui)
    };
    let (mut runtime, ids) = runtime(2);
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let requests = ids
        .iter()
        .enumerate()
        .map(|(seat, &client)| {
            let mut request = request(&mut runtime, ModuleId(seat as u16 + 1), client, program());
            request.entries = (0..30).collect();
            if ui {
                request.frame.client = None;
                request.context.console.seat = SeatId::new(seat as u8).unwrap();
            }
            request
        })
        .collect::<Vec<_>>();
    if ui {
        assert!(
            requests
                .iter()
                .all(|request| request.frame.client.is_none())
        );
    }
    let mut console = Console::new(Context::default()).unwrap();
    if !ui {
        let history = console
            .cvars
            .bind("cl_commandHistory", Context::default())
            .unwrap();
        console
            .cvars
            .write(
                history,
                &qa_core::command_history::DEFAULT_CAPACITY.to_string(),
            )
            .unwrap();
    }
    let mut host =
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, requests).unwrap();
    for (seat, &client) in ids.iter().enumerate() {
        host.local_clients[seat] = Some(client);
    }
    let mut peer_profile = profile;
    peer_profile.rules = RuleSetId::Quake2;
    peer_profile.image_settings =
        qa_render::material::resources::ImageSettings::native(RuleSetId::Quake2);
    host.runtime.model_assets = Some(
        ModelAssets::new(
            &host.runtime.vfs,
            assets,
            [profile, peer_profile],
            [world],
            Default::default(),
        )
        .unwrap(),
    );
    host.runtime
        .model_assets
        .as_mut()
        .unwrap()
        .bind_loaded_world(
            &host.runtime.vfs,
            "maps/probe.bsp",
            world,
            None,
            Bounds {
                mins: Vec3([-1.; 3]),
                maxs: Vec3([1.; 3]),
            },
            None,
            None,
        )
        .unwrap();
    host.local_worlds.fill(Some((world, presentation)));
    let mut seat_profiles = [[None, None, None, None, Some(profile)]; SeatId::COUNT];
    seat_profiles[1][RuleSetId::Quake3 as usize] = Some(peer_profile);
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &seat_profiles,
        320,
        200,
    )
    .unwrap();
    for (rules, &selected) in policy_rules.into_iter().zip(&policy_presentations) {
        host.local_worlds[0] = Some((world, selected));
        let view = host.client_views()[0].unwrap();
        assert_eq!(view.policy, Default::default());
        assert_eq!(
            view.near,
            if rules == RuleSetId::Quake2Rerelease {
                2.
            } else {
                4.
            }
        );
    }
    host.local_worlds[0] = Some((world, presentation));
    if !ui {
        assert_eq!(probe(&mut host, 0, 6, 0), 0);
    } // Cold selection, before any scene owner.
    assert!(probe(&mut host, 0, 0, 0) > 0);
    assert!(probe(&mut host, 1, 0, 0) > 0);
    host.bind_client_model_assets([]);
    let models = host.runtime.model_assets.as_ref().unwrap().catalog();
    let material = models
        .shader_outcome(
            b"ui/pic.tga",
            profile,
            qa_compat::services::ShaderLighting::TwoDimensional,
            None,
        )
        .unwrap()
        .shader;
    let peer_material = models
        .shader_outcome(
            b"ui/pic.tga",
            peer_profile,
            qa_compat::services::ShaderLighting::TwoDimensional,
            None,
        )
        .unwrap()
        .shader;
    assert_ne!(material, peer_material);
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            qa_render::Limits {
                polys: 2,
                vertices: 6,
                ..Default::default()
            },
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![loaded],
        )
        .unwrap()
        .0,
    );
    let _backend = admit_assets(&mut host);
    let curve_error = host
        .console
        .cvars
        .bind(
            "r_lodCurveError",
            Context {
                source: RuleSetId::Quake3,
                ..Default::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(curve_error, "37.5").unwrap();
    assert_eq!(host.client_views()[0].unwrap().curve_error, 37.5);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 1, 0), 0);
    assert_eq!(probe(&mut host, 0, 2, 0), 0);
    assert_eq!(probe(&mut host, 0, 2, 0), 0); // Both extra polygons rejected without a trap.
    if ui {
        assert_eq!(probe(&mut host, 0, 2, 0), 0);
        assert_eq!(probe(&mut host, 0, 2, 0), 0);
    }
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    assert_eq!(probe(&mut host, 0, 4, 0), 0);
    host.console.cvars.force_write(curve_error, "-1").unwrap();
    assert_eq!(probe(&mut host, 0, 10, 0), 0); // Change guest source after publication.
    assert_eq!(probe(&mut host, 0, 5, 0), 0); // Clear preserves 2D color and earlier view.
    if !ui {
        assert_eq!(probe(&mut host, 0, 11, 0), 0);
    } // Render the selected world, no longer NOWORLDMODEL.
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    assert_eq!(probe(&mut host, 0, 9, 0), 0); // A zero shader does not dereference a bad pointer.
    assert_eq!(probe(&mut host, 1, 4, 0), 0); // Other module retains white and separate target.
    let packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    assert_eq!(packet.rejected, 2);
    let Command::View(view) = packet.commands()[1] else {
        panic!("native view")
    };
    assert_eq!(view.refdef.time_ms, -12345);
    assert_eq!(view.refdef.curve_error, 37.5);
    assert_eq!(view.refdef.policy, qa_render::scene::ViewPolicy::default());
    assert_eq!(view.refdef.near, 4.);
    assert_eq!(view.refdef.text[7][1], 0xe7);
    assert_eq!(view.scene.polys.count, 2);
    let recipient = host.local_clients[0].map(|client| qa_render::effects::EffectClient {
        client,
        lifetime: host.runtime.server.clients[client.0 as usize].entity,
    });
    assert_eq!(
        view.particle_client, None,
        "no-world model previews omit world effects"
    );
    assert_eq!(
        packet.vertices(packet.polys(view.scene.polys)[0].vertices)[0]
            .position
            .0[0],
        8.
    );
    let Command::Draw2d(picture) = packet.commands()[2] else {
        panic!("native picture")
    };
    assert_eq!(picture.shader.source(host.native_shaders()), Some(material));
    assert_eq!(picture.rect, [0., 0., 90., 30.]);
    assert_eq!(picture.color, [127, 63, 255, 191]);
    let Command::View(second) = packet.commands()[3] else {
        panic!("second native view")
    };
    assert_eq!(second.refdef.time_ms, 777);
    assert_eq!(second.refdef.curve_error, -1.0);
    assert_eq!(second.particle_client, if ui { None } else { recipient });
    assert_eq!(packet.hidden_areas(second.hidden_areas), &[0xa5; 32]);
    let Command::Draw2d(peer) = packet.commands()[4] else {
        panic!("peer picture")
    };
    assert_eq!(peer.rect, [160., 0., 90., 30.]);
    assert_eq!(peer.color, [255; 4]);
    assert_eq!(
        peer.shader.source(host.native_shaders()),
        Some(peer_material)
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    // Both guest writes and submissions occur in one uninterrupted callback.
    assert_eq!(probe(&mut host, 0, 29, 0), 0);
    let same_callback = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    let captured = same_callback
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some((
                view.refdef.curve_error,
                view.refdef.model_lod.scale,
                view.refdef.model_lod.bias,
                view.refdef.policy,
                view.refdef.near,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    let fractional = qa_render::scene::ViewPolicy {
        draw_world: false,
        draw_entities: false,
        ..Default::default()
    };
    let enabled = qa_render::scene::ViewPolicy {
        ignore_pvs: true,
        ignore_areas: true,
        cull_bounds: false,
        ..Default::default()
    };
    assert_eq!(
        captured,
        [
            (12.5, 12.5, -2, fractional, 0.5),
            (-2.0, -2.0, 12, enabled, 2.)
        ]
    );
    assert!(
        host.runtime
            .scene
            .as_mut()
            .unwrap()
            .recycle(same_callback)
            .is_ok()
    );
    for name in ["r_novis", "r_nocull", "r_drawworld", "r_drawentities"] {
        let view = host
            .console
            .cvars
            .bind(
                name,
                Context {
                    source: RuleSetId::Quake3,
                    ..Default::default()
                },
            )
            .unwrap();
        host.console.cvars.force_write(view, "0.5").unwrap();
    }
    let q3 = host.client_views()[0].unwrap();
    assert_eq!(q3.policy, fractional);
    for &presentation in &policy_presentations[1..4] {
        host.local_worlds[1] = Some((world, presentation));
        assert_eq!(
            host.client_views()[1].unwrap().policy,
            qa_render::scene::ViewPolicy {
                ignore_pvs: true,
                cull_bounds: false,
                ..Default::default()
            }
        );
    }
    host.local_worlds[1] = Some((world, policy_presentations[4]));
    assert_eq!(host.client_views()[1].unwrap().policy, fractional);
    host.local_worlds[1] = Some((world, presentation));
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    assert_eq!(probe(&mut host, 1, 3, 0), 0);
    let packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    let views = packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(view),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(views.len(), 2);
    assert_eq!(views[0].refdef.policy, fractional);
    assert_eq!(
        views[1].refdef.policy,
        qa_render::scene::ViewPolicy {
            ignore_pvs: true,
            cull_bounds: false,
            ..Default::default()
        }
    );
    assert_eq!(views[0].refdef.viewport.x, 0);
    assert_eq!(views[1].refdef.viewport.x, 160);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}

fn named_scene_program(native: bool, ui: bool, name: &[u8]) -> Program {
    let mut initial = data();
    assert!(name.len() < 16);
    initial[16..32].fill(0);
    initial[16..16 + name.len()].copy_from_slice(name);
    if native {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            return native_scene_data(ui, &initial);
        }
    }
    qvm_scene_data(ui, &initial)
}

fn sun_probe(
    host: &mut FrameHost,
    module: ModuleId,
    client: Option<ClientId>,
    command: u32,
    value: u64,
    preparer: Option<&mut dyn qa_app::renderer::AssetPreparer>,
) -> u32 {
    scene_words(host, module, client, command, &[value], preparer).unwrap()
}

fn scene_words(
    host: &mut FrameHost,
    module: ModuleId,
    client: Option<ClientId>,
    command: u32,
    words: &[u64],
    preparer: Option<&mut dyn qa_app::renderer::AssetPreparer>,
) -> Result<u32, qa_session::dispatch::CallError> {
    host.call_module_export_with_driver(
        module,
        Export {
            client,
            ..Export::words(CallbackId(command), words)
        },
        ThinkTime::Milliseconds(1),
        &mut qa_app::modules::Driver::new(preparer),
    )?;
    Ok(
        match host.module_counts(module).unwrap().last_result.unwrap() {
            ModuleResult::Qvm(value) => value as u32,
            ModuleResult::Native(value) => value as u32,
            _ => panic!("word module"),
        },
    )
}

fn sun_direction(host: &mut FrameHost, module: ModuleId, client: Option<ClientId>) -> Vec3 {
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(sun_probe(host, module, client, 13, 0, None), 0);
    assert_eq!(sun_probe(host, module, client, 3, 0, None), 0);
    let composer = host.runtime.scene.as_mut().unwrap();
    let packet = composer.finish_frame().unwrap();
    let Command::View(view) = packet.commands()[1] else {
        panic!("sun lighting view")
    };
    assert_eq!(packet.rejected, 0);
    let direction = packet.entities(view.scene.entities)[0]
        .lighting
        .unwrap()
        .direction;
    assert!(composer.recycle(packet).is_ok());
    direction
}

fn assert_sun(actual: Vec3, expected: [f32; 3]) {
    for (actual, expected) in actual.0.into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 0.00001,
            "sun {actual} != {expected}"
        );
    }
}

fn sun_contexts(native: bool) {
    use qa_app::renderer::AssetPreparer;

    struct Backend {
        cpu: qa_render::cpu::CpuBackend,
        attempts: usize,
        reject_next: bool,
    }
    impl AssetPreparer for Backend {
        fn prepare_assets(
            &mut self,
            assets: &qa_render::Assets,
            scene: Option<qa_render::Limits>,
        ) -> Result<(), String> {
            self.attempts += 1;
            if std::mem::take(&mut self.reject_next) {
                return Err("reject this sun shader publication".into());
            }
            self.cpu
                .prepare_assets(assets, scene)
                .map_err(str::to_owned)
        }
    }

    let mut fixture = model_support::Fixture::new(&[
        ("models/test.md3", model_support::md3_surfaces()),
        (
            "textures/body.tga",
            model_support::tga(2, 2, &[[90, 20, 10, 255]; 4]),
        ),
        (
            "textures/glow.tga",
            model_support::tga(2, 2, &[[20, 10, 90, 255]; 4]),
        ),
        (
            "scripts/sun.shader",
            br#"
                sun/a { q3map_sun 1 0 0 100 0 0 { map $whiteimage } }
                sun/b { q3map_sun 0 1 0 100 90 0 { map $whiteimage } }
                sun/c { q3map_sun 1 1 0 100 180 0 { map $whiteimage } }
                sun/d { q3map_sun 0 0 1 100 0 90 bogus }
            "#
            .to_vec(),
        ),
    ]);
    let (mut runtime, clients) = runtime(2);
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    // UI shares rendering with its captured seat, without a network binding.
    let modules = [
        (false, Some(clients[0]), 0, b"sun/a".as_slice()),
        (true, None, 0, b"sun/b".as_slice()),
        (true, None, 0, b"sun/d".as_slice()),
        (true, None, 0, b"sun/c".as_slice()),
        (false, Some(clients[1]), 1, b"sun/a".as_slice()),
        (true, None, 2, b"sun/a".as_slice()),
    ];
    let requests = modules
        .iter()
        .enumerate()
        .map(|(index, &(ui, client, seat, name))| {
            let mut row = request(
                &mut runtime,
                ModuleId(index as u16 + 1),
                client.unwrap_or(clients[0]),
                named_scene_program(native, ui, name),
            );
            row.entries = (0..26).collect();
            row.frame.client = client;
            row.context.console.seat = SeatId::new(seat).unwrap();
            row
        })
        .collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.local_clients[..2].copy_from_slice(&clients.iter().copied().map(Some).collect::<Vec<_>>());
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut peer_profile = profile;
    peer_profile.rules = RuleSetId::Quake2;
    peer_profile.image_settings =
        qa_render::material::resources::ImageSettings::native(RuleSetId::Quake2);
    host.runtime.model_assets = Some(
        ModelAssets::new(
            &host.runtime.vfs,
            qa_render::Assets::load().unwrap(),
            [profile, peer_profile],
            [],
            Default::default(),
        )
        .unwrap(),
    );
    let mut seat_profiles = [[None, None, None, None, Some(profile)]; SeatId::COUNT];
    seat_profiles[1][RuleSetId::Quake3 as usize] = Some(peer_profile);
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &seat_profiles,
        320,
        200,
    )
    .unwrap();
    for (index, &(_, client, _, _)) in modules.iter().enumerate() {
        let id = ModuleId(index as u16 + 1);
        let model = sun_probe(&mut host, id, client, 12, 0, None);
        assert!(model > 0);
        assert_eq!(sun_probe(&mut host, id, client, 14, model as u64, None), 0);
    }
    // Native logical admission does not require a SceneComposer to exist.
    assert!(host.runtime.scene.is_none());
    let a = sun_probe(&mut host, ModuleId(1), Some(clients[0]), 0, 0, None);
    let b = sun_probe(&mut host, ModuleId(2), None, 0, 0, None);
    assert!(a > 0 && b > 0 && a != b);
    assert_eq!(
        sun_probe(&mut host, ModuleId(5), Some(clients[1]), 0, 0, None),
        a
    );
    assert_eq!(sun_probe(&mut host, ModuleId(6), None, 0, 0, None), a);
    assert_eq!(
        sun_probe(&mut host, ModuleId(1), Some(clients[0]), 0, 0, None),
        a
    );
    host.bind_client_model_assets([]);
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            Default::default(),
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![],
        )
        .unwrap()
        .0,
    );
    let mut backend = Backend {
        cpu: admit_assets(&mut host),
        attempts: 0,
        reject_next: false,
    };
    assert_sun(
        sun_direction(&mut host, ModuleId(1), Some(clients[0])),
        [0., 1., 0.],
    );
    assert_sun(sun_direction(&mut host, ModuleId(2), None), [0., 1., 0.]);
    assert_sun(
        sun_direction(&mut host, ModuleId(5), Some(clients[1])),
        [1., 0., 0.],
    );
    assert_sun(sun_direction(&mut host, ModuleId(6), None), [1., 0., 0.]);

    // A reached native prefix applies even when public registration returns zero.
    assert_eq!(
        sun_probe(&mut host, ModuleId(3), None, 0, 0, Some(&mut backend)),
        0
    );
    assert_eq!(backend.attempts, 1);
    assert_sun(
        sun_direction(&mut host, ModuleId(1), Some(clients[0])),
        [0., 0., 1.],
    );
    let retained = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    backend.reject_next = true;
    assert_eq!(
        sun_probe(&mut host, ModuleId(4), None, 0, 0, Some(&mut backend)),
        0
    );
    assert_eq!(backend.attempts, 2);
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        retained
    );
    assert_eq!(backend.cpu.admitted_assets(), Some(retained));
    assert_sun(sun_direction(&mut host, ModuleId(2), None), [0., 0., 1.]);
    assert!(sun_probe(&mut host, ModuleId(4), None, 0, 0, Some(&mut backend)) > 0);
    assert_eq!(backend.attempts, 3);
    assert_sun(
        sun_direction(&mut host, ModuleId(1), Some(clients[0])),
        [-1., 0., 0.],
    );

    host.runtime.vfs.take_lookup_count();
    assert_eq!(sun_probe(&mut host, ModuleId(3), None, 0, 0, None), 0);
    assert_eq!(
        sun_probe(&mut host, ModuleId(1), Some(clients[0]), 0, 0, None),
        a
    );
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    assert_eq!(backend.attempts, 3);
    assert_sun(sun_direction(&mut host, ModuleId(2), None), [-1., 0., 0.]);
    assert_sun(
        sun_direction(&mut host, ModuleId(5), Some(clients[1])),
        [1., 0., 0.],
    );
    assert_sun(sun_direction(&mut host, ModuleId(6), None), [1., 0., 0.]);
    for index in 1..=modules.len() {
        assert_eq!(host.module_counts(ModuleId(index as u16)).unwrap().traps, 0);
    }
}

pub(super) fn ownership() {
    use qa_app::renderer::{Context as RenderContext, Renderer};
    use qa_render::material::resources::ImageSettings;
    let mut fixture = model_support::Fixture::new(&[
        (
            "gfx/2d/bigchars.tga",
            model_support::tga(128, 128, &vec![[255; 4]; 128 * 128]),
        ),
        (
            "gfx/2d/conback.tga",
            model_support::tga(2, 2, &[[30, 40, 50, 255]; 4]),
        ),
        (
            "ui/pic.tga",
            model_support::tga(2, 2, &[[90, 20, 10, 255]; 4]),
        ),
        (
            "scripts/graphics.shader",
            b"white { { map $whiteimage } } console { { map gfx/2d/conback.tga } }".to_vec(),
        ),
    ]);
    let (mut runtime, ids) = runtime(2);
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let requests = ids
        .iter()
        .enumerate()
        .map(|(seat, &client)| {
            let mut request = request(
                &mut runtime,
                ModuleId(seat as u16 + 1),
                client,
                qvm_scene(seat == 0),
            );
            request.frame.callback = CallbackId(3);
            if seat == 0 {
                request.frame.client = None;
            }
            request
        })
        .collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.local_clients[..2].copy_from_slice(&ids.iter().copied().map(Some).collect::<Vec<_>>());
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut assets = qa_render::Assets::load().unwrap();
    let world = model_support::world(&mut assets, -1.);
    let presentation = qa_render::world::WorldPresentationId(0);
    host.local_worlds[..2].fill(Some((world, presentation)));
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    let mut owner =
        ModelAssets::new(&host.runtime.vfs, assets, [], [world], Default::default()).unwrap();
    let console = qa_app::renderer::console::ConsoleAssets::load(
        &host.runtime.vfs,
        &mut owner,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        qa_render::material::ShaderCompilePolicy::default(),
        320,
        200,
    )
    .unwrap();
    let font_registration = console.font_assets().registration.unwrap();
    assert!(font_registration.defaulted.is_none());
    // Prepare the guest resource physically without creating a native logical node.
    assert!(
        qa_compat::services::ModelLoader::register_shader(
            &mut owner,
            &host.runtime.vfs,
            b"ui/pic.tga",
            RuleSetId::Quake3,
            true,
            qa_compat::services::ShaderLighting::TwoDimensional,
        )
        .unwrap()
            > 0
    );
    let effects = std::sync::Arc::clone(&host.runtime.catalog.registry.effects);
    let bindings = vec![None; effects.len() + 1];
    let effects = qa_render::effects::Effects::load(effects, 8, 2, &bindings).unwrap();
    let configuration = std::sync::Arc::new(qa_core::presentation::RendererConfiguration {
        display: qa_core::presentation::DisplayConfiguration {
            width: 320,
            height: 200,
            ..Default::default()
        },
        ..Default::default()
    });
    let loaded = LoadedWorld {
        world,
        presentations: vec![LoadedWorldPresentation {
            id: presentation,
            profile: WorldPresentationProfile::native(RuleSetId::Quake3, None),
            lighting: Default::default(),
            compile_policy: Default::default(),
            defaults: Default::default(),
        }]
        .into_boxed_slice(),
        diagnostics: Box::default(),
    };
    host.runtime.model_assets = Some(owner);
    let huds = std::array::from_fn(|_| None);
    let model_choices = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .catalog()
        .max_choice_nodes();
    let (assets, staged) = host.stage_engine_graphics(&console, &huds).unwrap();
    let (renderer, scene) = Renderer::load(
        RenderContext::Cpu(configuration),
        320,
        200,
        assets,
        vec![loaded],
        Some(qa_render::cpu::RasterBands::One),
        console,
        huds,
        effects,
        model_choices,
    )
    .unwrap();
    staged.commit();
    host.bind_client_model_assets([]);
    host.publish_model_admission(renderer.admitted_assets().unwrap())
        .unwrap();
    host.runtime.scene = Some(scene);
    struct Source {
        renderer: Renderer,
        views: [Option<qa_app::host::ClientView>; SeatId::COUNT],
    }
    impl qa_app::host::FrameSource for Source {
        fn begin_frame(&mut self) -> Duration {
            Duration::from_millis(16)
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            queue
                .push(SysEvent {
                    time: EventTime::from_milliseconds(16),
                    kind: EventKind::Time,
                })
                .unwrap();
        }
        fn wait_time(&mut self, _: Duration) -> Duration {
            self.begin_frame()
        }
        fn elapsed(&self) -> Duration {
            Duration::ZERO
        }
        fn present(&mut self) {}
        fn render(
            &mut self,
            scene: Option<&mut qa_app::renderer::SceneComposer>,
            resources: Option<qa_app::model_assets::ModelView<'_>>,
            native: &dyn qa_render::shader::reference::NativeShaders,
            _: &[Option<qa_app::host::ClientView>; SeatId::COUNT],
            _: &[Option<qa_app::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
            _: &[Option<qa_app::renderer::hud::HudView<'_>>; SeatId::COUNT],
            console: qa_console::presentation::ConsoleView<'_>,
        ) {
            self.renderer.frame(
                scene.unwrap(),
                resources.unwrap(),
                native,
                &self.views,
                &std::array::from_fn(|_| None),
                &std::array::from_fn(|_| None),
                console,
            );
        }
    }
    let view = qa_app::host::ClientView {
        world,
        world_presentation: presentation,
        presentation: Default::default(),
        presentation_rules: RuleSetId::Quake3,
        origin: Vec3::default(),
        angles: Vec3::default(),
        fov_x: 90.,
        screen_blend: [0.0; 4],
        damage_blend: [0.0; 4],
        flags: Default::default(),
        time_ms: 16,
        model_time: None,
        lighting_scales: [0.6, 1.],
        curve_error: qa_render::scene::DEFAULT_CURVE_ERROR,
        model_lod: Default::default(),
        model_replacements: Default::default(),
        policy: Default::default(),
        near: 4.,
    };
    let mut source = Source {
        renderer,
        views: [Some(view), Some(view), None, None],
    };
    let result = host.frame(&mut source, true);
    assert!(!host.runtime.quit);
    assert_eq!(result.drains, 2);
    assert_eq!(
        source.renderer.sample.stats.views, 3,
        "one native client view, one builtin peer, one renderer-only UI preview"
    );
    assert_eq!(source.renderer.sample.stats.rejected, 0);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);

    use qa_render::shader::{
        reference::ShaderRef,
        registration::{ShaderOwner, ShaderResource},
    };
    // These already-running fixture modules create their renderer contexts on
    // their first actual registration. Engine graphics replay before that guest node.
    let graphics_prefix = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    host.runtime.vfs.take_lookup_count();
    let handle = probe(&mut host, 0, 0, 0);
    assert!(
        handle > 3,
        "font, white and console must precede guest shader nodes"
    );
    let peer_handle = probe(&mut host, 1, 0, 0);
    assert!(peer_handle > 3);
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        graphics_prefix
    );
    let lifetime = host.runtime.server.clients[ids[0].0 as usize].entity;
    let owner = ShaderOwner::Client(ids[0], lifetime);
    let context = host.native_shaders().context(owner, profile, None).unwrap();
    let font_nodes = host
        .native_shaders()
        .resource_nodes(context, ShaderResource::Graphic(font_registration.key))
        .unwrap();
    assert_eq!(font_nodes.len(), 1);
    let font_ref = qa_render::shader::reference::NativeShaderRef {
        context,
        node: font_nodes[0],
    };
    assert_eq!(
        host.native_shaders().source(font_ref).unwrap().shader,
        font_registration.shader
    );
    let peer_lifetime = host.runtime.server.clients[ids[1].0 as usize].entity;
    let peer_context = host
        .native_shaders()
        .context(ShaderOwner::Client(ids[1], peer_lifetime), profile, None)
        .unwrap();
    assert_ne!(context, peer_context);

    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    probe(&mut host, 0, 4, 0);
    let mut packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    let draw = packet
        .commands()
        .iter()
        .find_map(|command| match command {
            Command::Draw2d(draw) => Some(draw),
            _ => None,
        })
        .unwrap();
    let ShaderRef::Native(reference) = draw.shader else {
        panic!("native picture lost its renderer context")
    };
    assert_eq!(reference.context, context);
    assert_eq!(reference.node.0 + 1, handle);
    let physical = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .catalog()
        .shader_outcome(
            b"ui/pic.tga",
            profile,
            qa_compat::services::ShaderLighting::TwoDimensional,
            None,
        )
        .unwrap()
        .shader;
    assert_eq!(draw.shader.source(host.native_shaders()), Some(physical));
    packet.resolve_shaders(
        host.runtime.model_assets.as_ref().unwrap().assets(),
        host.native_shaders(),
    );
    assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
}

fn entity_host(
    native: bool,
    ui: bool,
    fixture: &mut model_support::Fixture,
) -> (FrameHost, qa_app::model_assets::ModelProfile) {
    entity_host_program(
        ui,
        fixture,
        named_scene_program(native, ui, b"textures/glow"),
        qa_render::Assets::load().unwrap(),
        ModelRequest::native("", RuleSetId::Quake3, None).profile(),
        Protocol::Quake3_68,
    )
}

fn entity_host_program(
    ui: bool,
    fixture: &mut model_support::Fixture,
    program: Program,
    assets: qa_render::Assets,
    profile: qa_app::model_assets::ModelProfile,
    protocol: Protocol,
) -> (FrameHost, qa_app::model_assets::ModelProfile) {
    let (mut runtime, ids) = runtime_protocol(1, protocol);
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let mut request = request(&mut runtime, ModuleId(1), ids[0], program);
    request.entries = (0..20).collect();
    if ui {
        request.frame.client = None;
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    host.local_clients[0] = Some(ids[0]);
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    host.runtime.model_assets = Some(
        ModelAssets::new(&host.runtime.vfs, assets, [profile], [], Default::default()).unwrap(),
    );
    (host, profile)
}

pub(super) fn entities(native: bool, ui: bool) {
    let mut fixture = model_support::Fixture::surfaces();
    let (mut host, profile) = entity_host(native, ui, &mut fixture);
    let model = probe(&mut host, 0, 12, 0);
    assert!(model > 0);
    assert_eq!(probe(&mut host, 0, 14, model as u64), 0);
    let shader = probe(&mut host, 0, 0, 0);
    assert!(shader > 0);
    let skin = probe(&mut host, 0, 16, 0);
    assert!(skin > 0);
    host.bind_client_model_assets([]);
    let models = host.runtime.model_assets.as_ref().unwrap().catalog();
    let registration = models.renderer_registration(model as i32, profile).unwrap();
    let skinned = models
        .bindings(
            registration,
            0,
            qa_app::model_assets::ModelMaterialPolicy {
                custom_skin: skin as i32,
                ..Default::default()
            },
        )
        .unwrap()
        .collect::<Vec<_>>();
    let override_material = models
        .shader_outcome(
            b"textures/glow",
            profile,
            qa_compat::services::ShaderLighting::TwoDimensional,
            None,
        )
        .unwrap()
        .shader;
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            qa_render::Limits {
                entities: 2,
                ..Default::default()
            },
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![],
        )
        .unwrap()
        .0,
    );
    let _backend = admit_assets(&mut host);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 13, 0), 0); // A full two-surface model fails atomically.
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    assert_eq!(packet.rejected, 1);
    let Command::View(view) = packet.commands()[1] else {
        panic!("model view")
    };
    assert_eq!(view.scene.entities.count, 2);
    for entity in packet.entities(view.scene.entities) {
        assert_eq!(entity.frame, 1);
        assert_eq!(entity.old_frame, 0);
        assert_eq!(entity.back_lerp, 0.25);
        assert_eq!(entity.color, [201, 150, 90, 255]);
        assert_eq!(entity.shader_time, 1.25);
        assert_eq!(entity.shader_texcoord, [0.125, 0.5]);
        assert!(
            entity.lighting.is_some(),
            "native no-world model lighting reaches the shared stage inputs"
        );
    }
    assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
    assert_eq!(probe(&mut host, 0, 16, 0), skin);
    assert_eq!(probe(&mut host, 0, 17, skin as u64), 0);
    assert_eq!(probe(&mut host, 0, 18, 0), 0);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    let Command::View(view) = packet.commands()[1] else {
        panic!("skin view")
    };
    for (entity, binding) in packet.entities(view.scene.entities).iter().zip(&skinned) {
        assert_eq!(
            entity
                .shader
                .and_then(|shader| shader.source(host.native_shaders())),
            Some(binding.shader)
        );
    }
    assert_eq!(packet.rejected, 0);
    assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
    // A custom shader takes priority; an invalid positive handle uses the default.
    for (handle, expected) in [
        (shader as u64, override_material),
        (
            i32::MAX as u64,
            host.runtime
                .model_assets
                .as_ref()
                .unwrap()
                .catalog()
                .native_default(profile)
                .unwrap(),
        ),
    ] {
        assert_eq!(probe(&mut host, 0, 18, handle), 0);
        assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
        assert_eq!(probe(&mut host, 0, 13, 0), 0);
        assert_eq!(probe(&mut host, 0, 3, 0), 0);
        let packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
        let Command::View(view) = packet.commands()[1] else {
            panic!("override view")
        };
        assert!(packet.entities(view.scene.entities).iter().all(|entity| {
            entity
                .shader
                .and_then(|shader| shader.source(host.native_shaders()))
                == Some(expected)
        }));
        assert_eq!(packet.rejected, 0);
        assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
    }
    assert_eq!(probe(&mut host, 0, 18, shader as u64), 0);
    for (kind, triangles) in [(1, 0), (2, 2), (3, 12), (4, 2), (5, 2), (6, 8), (7, 0)] {
        assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
        assert_eq!(probe(&mut host, 0, 15, kind), 0);
        assert_eq!(probe(&mut host, 0, 13, 0), 0);
        assert_eq!(probe(&mut host, 0, 3, 0), 0);
        let packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
        let Command::View(view) = packet.commands()[1] else {
            panic!("generated view")
        };
        assert_eq!(view.scene.entities.count, 1);
        assert_eq!(view.scene.polys.count, triangles);
        if kind == 3 {
            assert_eq!(packet.entity(0).origin, Vec3::default());
            assert_eq!(packet.entity(0).color, [255, 0, 0, 255]);
            assert_eq!(
                packet.entity(0).model,
                qa_render::SceneModel::Beam {
                    end: Vec3([-8., 0., 0.]),
                    radius: 4.,
                    world_space: true,
                }
            );
        }
        assert_eq!(packet.rejected, 0);
        assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
    }
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}

pub(super) fn sprite_entities(native: bool, ui: bool) {
    let mut fixture = model_support::Fixture::new(&[("models/test.spr", model_support::spr())]);
    let mut initial = data();
    initial[880..896].copy_from_slice(b"models/test.spr\0");
    initial[780..784].copy_from_slice(&99i32.to_le_bytes());
    let program = if native {
        #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
        {
            native_scene_data(ui, &initial)
        }
        #[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
        {
            qvm_scene_data(ui, &initial)
        }
    } else {
        qvm_scene_data(ui, &initial)
    };
    let mut assets = qa_render::Assets::load().unwrap();
    let palette = model_support::palette(&mut assets);
    let profile = ModelRequest::native("", RuleSetId::Quake3, Some(palette)).profile();
    let models_before = assets.counts().models;
    let (mut host, profile) = entity_host_program(
        ui,
        &mut fixture,
        program,
        assets,
        profile,
        Protocol::Quake3_68,
    );
    let handle = probe(&mut host, 0, 12, 0);
    assert!(handle > 0);
    assert_eq!(probe(&mut host, 0, 14, handle as u64), 0);
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let registration = owner
        .catalog()
        .renderer_registration(handle as i32, profile)
        .unwrap();
    let choice = owner
        .catalog()
        .bindings(registration, 0, Default::default())
        .unwrap()
        .choices()
        .next()
        .unwrap();
    let model = choice.model;
    assert_eq!(
        (
            owner.assets().model(model).unwrap().frame_count(),
            owner.assets().model(model).unwrap().native_frame_count()
        ),
        (4, 3)
    );
    let expected = choice
        .resolve_shader(
            choice.shaders().next().unwrap(),
            qa_render::shader::reference::ShaderView::direct(),
            None,
        )
        .unwrap()
        .source(&())
        .unwrap();
    assert_eq!(owner.assets().counts().models, models_before + 1);
    host.bind_client_model_assets([]);
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            qa_render::Limits::default(),
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![],
        )
        .unwrap()
        .0,
    );
    let _backend = admit_assets(&mut host);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    let Command::View(view) = packet.commands()[1] else {
        panic!("native SPR view")
    };
    assert_eq!((view.scene.entities.count, view.scene.polys.count), (1, 2));
    assert!(matches!(
        packet.entity(0).model,
        qa_render::SceneModel::Choice { .. }
    ));
    for draw in packet.draws(view.scene.draws) {
        assert_eq!(draw.mesh, Some(model));
        assert_eq!(
            draw.frames,
            Some(qa_render::scene::ModelFramePose {
                current: 0,
                previous: 0,
                back_lerp: 0.,
            })
        );
        assert_eq!(
            draw.shader.unwrap().source(host.native_shaders()),
            Some(expected)
        );
    }
    assert_eq!(packet.rejected, 0);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
}

pub(super) fn lod_entities(native: bool, ui: bool) {
    let mut fixture = model_support::Fixture::new(&[
        (
            "models/test.md3",
            model_support::md3_named_surfaces(&[
                (b"body_1", &[b"textures/body", b"textures/alternate"]),
                (b"near_1", &[b"textures/alternate"]),
            ]),
        ),
        (
            "models/test_1.md3",
            model_support::md3_named_surfaces(&[
                (b"coarse_1", &[b"textures/glow"]),
                (b"cap_1", &[b"textures/body"]),
                (b"tail_1", &[b"textures/alternate"]),
            ]),
        ),
        (
            "textures/body.tga",
            model_support::tga(2, 2, &[[90, 20, 10, 255]; 4]),
        ),
        (
            "textures/alternate.tga",
            model_support::tga(2, 2, &[[10, 90, 20, 255]; 4]),
        ),
        (
            "textures/glow.tga",
            model_support::tga(2, 2, &[[20, 10, 90, 255]; 4]),
        ),
    ]);
    let (mut host, profile) = entity_host(native, ui, &mut fixture);
    let handle = probe(&mut host, 0, 12, 0);
    assert!(handle > 0);
    assert_eq!(probe(&mut host, 0, 14, handle as u64), 0);
    host.bind_client_model_assets([]);
    let models = host.runtime.model_assets.as_ref().unwrap().catalog();
    let registration = models
        .renderer_registration(handle as i32, profile)
        .unwrap();
    let lods = models
        .lod_bindings(registration, 0, Default::default())
        .unwrap();
    assert_eq!(lods.native_count(), 2);
    let mut expected: [Vec<_>; 2] = std::array::from_fn(|_| Vec::new());
    for parts in lods {
        for (level, expected) in expected.iter_mut().enumerate() {
            if let Some(choice) = &parts[level] {
                let binding = choice.physical_binding().unwrap();
                expected.push((binding.model, binding.shader));
            }
        }
    }
    assert_eq!(expected[0].len(), 2);
    assert_eq!(expected[1].len(), 3);
    for level in &mut expected {
        level.sort_by_key(|(model, _)| model.0);
    }
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            qa_render::Limits {
                entities: 6,
                ..Default::default()
            },
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![],
        )
        .unwrap()
        .0,
    );
    let _backend = admit_assets(&mut host);
    let context = Context {
        source: RuleSetId::Quake3,
        ..Default::default()
    };
    let scale = host.console.cvars.bind("r_lodscale", context).unwrap();
    let bias = host.console.cvars.bind("r_lodbias", context).unwrap();
    host.console.cvars.force_write(scale, "20").unwrap();
    host.console.cvars.force_write(bias, "0").unwrap();
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    host.console.cvars.force_write(bias, "1").unwrap();
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    let views = packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(view),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(views.len(), 2);
    for (level, view) in views.iter().enumerate() {
        assert_eq!(view.refdef.model_lod.bias, level as i32);
        let mut draws = packet
            .draws(view.scene.draws)
            .iter()
            .map(|item| {
                (
                    item.mesh.unwrap(),
                    item.shader.unwrap().source(host.native_shaders()).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        draws.sort_by_key(|(model, _)| model.0);
        assert_eq!(draws, expected[level]);
    }
    assert_eq!(packet.rejected, 0);
    assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}

pub(super) fn inline_entities(native: bool) {
    let mut fixture = model_support::Fixture::new(&[(
        "scripts/inline.shader",
        b"ui/pic { q3map_sun 0 1 0 100 90 0 { map $whiteimage } }".to_vec(),
    )]);
    let (mut runtime, ids) = runtime(1);
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let program = if native {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            native_scene(false)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            qvm_scene(false)
        }
    } else {
        qvm_scene(false)
    };
    let mut request = request(&mut runtime, ModuleId(1), ids[0], program);
    request.entries = (0..20).collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut assets = qa_render::Assets::load().unwrap();
    let portal_material = assets
        .register_material(
            "native portal",
            &[qa_render::assets::Stage {
                alpha_gen: qa_render::shader::AlphaGen::Const(0.),
                blend: Some(qa_render::shader::StageBlend::ALPHA),
                depth_write: false,
                ..Default::default()
            }],
            qa_render::assets::MaterialSettings {
                sort: 1.,
                portal: true,
                cull: qa_render::shader::Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let world = model_support::world_mutated(
        &mut assets,
        10.,
        &[&[qa_render::world::SurfaceMaterial {
            shader: qa_render::ShaderBinding::material(
                portal_material,
                qa_render::world::geometry::LightSource::None,
            ),
            ..Default::default()
        }]],
        |geometry| {
            use qa_render::world::geometry::*;
            let bounds = qa_core::primitives::Bounds {
                mins: Vec3([8., -8., -8.]),
                maxs: Vec3([8., 8., 8.]),
            };
            geometry.vertices = [[8., -8., -8.], [8., 8., -8.], [8., 0., 8.]]
                .map(|position| WorldVertex {
                    vertex: qa_render::Vertex {
                        position: Vec3(position),
                        normal: Vec3([-1., 0., 0.]),
                        ..Default::default()
                    },
                })
                .into();
            geometry.indices = vec![0, 1, 2];
            geometry.boundaries = vec![qa_core::primitives::IndexSpan { first: 0, count: 3 }];
            geometry.surfaces = vec![WorldSurface {
                source_id: 0,
                kind: GeometryKind::Triangles,
                vertices: qa_core::primitives::IndexSpan { first: 0, count: 3 },
                indices: qa_core::primitives::IndexSpan { first: 0, count: 3 },
                boundaries: qa_core::primitives::IndexSpan { first: 0, count: 1 },
                plane: Some(qa_core::primitives::Plane::oriented(
                    Vec3([-1., 0., 0.]),
                    -8.,
                )),
                bounds,
                texture_coordinates: TextureCoordinates::Normalized,
                texture_projection: [[0.; 4]; 2],
                texture_minima: [0; 2],
                texture_extents: [0; 2],
                lightmap_projection: None,
                lightmap_grid: [0; 2],
                styles: [255; 4],
                light_source: LightSource::None,
                light_encoding: LightEncoding::Rgb,
                light_samples: Default::default(),
                source_texture: None,
                source_texture_info: None,
                source_shader: None,
                source_flags: 0,
                source_contents: 0,
                no_draw: false,
                source_fog: -1,
                source_brush_side: -1,
                source_lightmap: -1,
                lightmap_rect: [0; 4],
                lightmap_origin: Vec3::default(),
                lightmap_vectors: [Vec3::default(); 3],
                patch: None,
            }];
            geometry.models[0].surfaces = qa_core::primitives::IndexSpan { first: 0, count: 1 };
        },
    );
    let presentation = qa_render::world::WorldPresentationId(0);
    let loaded = LoadedWorld {
        world,
        presentations: vec![LoadedWorldPresentation {
            id: presentation,
            profile: WorldPresentationProfile::native(RuleSetId::Quake3, None),
            lighting: Default::default(),
            compile_policy: Default::default(),
            defaults: Default::default(),
        }]
        .into_boxed_slice(),
        diagnostics: Box::default(),
    };
    assets
        .register_world_presentation_metadata(world, &loaded.presentations, None)
        .unwrap();
    let bounds = assets.world(world).unwrap().geometry().models[1].bounds;
    let grid = qa_render::lighting::LightGrid::load(
        qa_core::primitives::Bounds {
            mins: Vec3::default(),
            maxs: Vec3([1.; 3]),
        },
        Vec3([1.; 3]),
        [qa_formats::bsp::LightGridPoint {
            ambient: [10, 20, 30],
            directed: [0; 3],
            longitude: 0,
            latitude: 0,
        }; 8]
            .into_iter(),
        0,
    )
    .unwrap();
    assets.set_light_grid(world, Some(grid)).unwrap();
    let meshes = assets.models().len();
    let mut builder = ModelAssets::new(
        &host.runtime.vfs,
        assets,
        [profile],
        [world],
        Default::default(),
    )
    .unwrap();
    builder
        .bind_loaded_world(
            &host.runtime.vfs,
            "maps/probe.bsp",
            world,
            None,
            bounds,
            None,
            None,
        )
        .unwrap();
    host.runtime.model_assets = Some(builder);
    host.local_clients[0] = Some(ids[0]);
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    // No scene world is inferred just because geometry exists in the catalog.
    assert_eq!(probe(&mut host, 0, 19, 0), 0);
    assert_eq!(probe(&mut host, 0, 6, 0), 0);
    let model = probe(&mut host, 0, 19, 0);
    assert!(model > 0);
    assert!(probe(&mut host, 0, 0, 0) > 0);
    host.bind_client_model_assets([]);
    let models = host.runtime.model_assets.as_ref().unwrap().catalog();
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .models()
            .len(),
        meshes
    );
    assert_eq!(models.model_bounds(model as i32), Some(bounds));
    let binding = models.resolve_inline(world, 1).unwrap();
    host.local_clients[0] = Some(ids[0]);
    host.local_worlds[0] = Some((world, presentation));
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            Default::default(),
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![loaded],
        )
        .unwrap()
        .0,
    );
    let _backend = admit_assets(&mut host);
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    assert_eq!(probe(&mut host, 0, 19, 0), model);
    assert_eq!(probe(&mut host, 0, 14, model as u64), 0);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let owner = host.runtime.scene.as_mut().unwrap();
    let packet = owner.finish_frame().unwrap();
    let Command::View(view) = packet.commands()[1] else {
        panic!("inline view")
    };
    assert_eq!(packet.rejected, 0);
    assert_eq!(view.scene.entities.count, 1);
    let entity = packet.entities(view.scene.entities)[0];
    assert_eq!(
        entity.model,
        qa_render::scene::SceneModel::World {
            world,
            presentation,
            surfaces: binding.surfaces
        }
    );
    assert_eq!(entity.origin.0, [8., 0., 0.]);
    assert_sun(entity.lighting.unwrap().direction, [0., 1., 0.]);
    assert_eq!(
        entity.axes.map(|axis| axis.0),
        [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
    );
    assert!(owner.recycle(packet).is_ok());
    assert_eq!(probe(&mut host, 0, 11, 0), 0); // Enable world lighting in the next refdef.
    assert_eq!(probe(&mut host, 0, 18, i32::MAX as u64), 0);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let owner = host.runtime.scene.as_mut().unwrap();
    let packet = owner.finish_frame().unwrap();
    let Command::View(view) = packet.commands()[1] else {
        panic!("grid view")
    };
    assert_eq!(
        packet.entities(view.scene.entities)[0].shader,
        None,
        "native inline surfaces ignore customShader, including invalid handles"
    );
    assert_eq!(
        packet.entities(view.scene.entities)[0]
            .lighting
            .unwrap()
            .ambient,
        [38., 44., 50.]
    );
    assert_eq!(packet.rejected, 0);
    assert!(owner.recycle(packet).is_ok());
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 15, 7), 0);
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let owner = host.runtime.scene.as_mut().unwrap();
    let packet = owner.finish_frame().unwrap();
    let [
        Command::Clear(_),
        Command::View(child),
        Command::View(parent),
    ] = packet.commands()
    else {
        panic!("native portal views")
    };
    assert!(child.refdef.portal && child.refdef.clip_plane.is_some());
    assert!(!parent.refdef.portal);
    assert_eq!(parent.refdef.time_ms, child.refdef.time_ms);
    assert_eq!(
        packet.entities(parent.scene.entities)[0].model,
        packet.entities(child.scene.entities)[0].model
    );
    assert_eq!(packet.rejected, 0);
    assert!(owner.recycle(packet).is_ok());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}

fn mark_fragments(native: bool) {
    use qa_render::world::{
        SurfaceMaterial,
        geometry::{GeometryKind, WorldModel},
    };
    let (mut geometry, visibility) = mark_world::inputs(1);
    geometry.surfaces[0].kind = GeometryKind::Polygon;
    geometry.surfaces[0].plane = Some(Plane::oriented(Vec3([-1., 0., 0.]), -1.));
    geometry.models.push(WorldModel {
        bounds: geometry.surfaces[0].bounds,
        origin: Vec3::default(),
        surfaces: IndexSpan { first: 0, count: 1 },
    });
    let mut assets = qa_render::Assets::load().unwrap();
    let world = assets
        .register_world(
            geometry,
            visibility,
            &[qa_render::world::WorldPresentationInput::new(&[
                SurfaceMaterial::default(),
            ])],
        )
        .unwrap();
    let presentation = assets.world(world).unwrap().presentations()[0];
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let (mut runtime, ids) = runtime(1);
    let program = if native {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            native_scene(false)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            qvm_scene(false)
        }
    } else {
        qvm_scene(false)
    };
    let mut request = request(&mut runtime, ModuleId(1), ids[0], program);
    request.entries = (0..26).collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    host.local_clients[0] = Some(ids[0]);
    host.local_worlds[0] = Some((world, presentation));
    host.runtime.model_assets = Some(
        ModelAssets::new(
            &host.runtime.vfs,
            assets,
            [profile],
            [world],
            Default::default(),
        )
        .unwrap(),
    );
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    assert_eq!(probe(&mut host, 0, 20, 0), 0); // No scene owner yet.
    host.bind_client_model_assets([]);
    let loaded = LoadedWorld {
        world,
        presentations: vec![LoadedWorldPresentation {
            id: presentation,
            profile: WorldPresentationProfile::native(RuleSetId::Quake3, None),
            lighting: Default::default(),
            compile_policy: Default::default(),
            defaults: Default::default(),
        }]
        .into_boxed_slice(),
        diagnostics: Box::default(),
    };
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            qa_render::Limits::default(),
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![loaded],
        )
        .unwrap()
        .0,
    );
    assert_eq!(probe(&mut host, 0, 20, 0), 0); // An unpublished suffix is not renderer authority.
    let _backend = admit_assets(&mut host);
    assert_eq!(probe(&mut host, 0, 20, 0), 1);
    assert_eq!(probe(&mut host, 0, 21, 0), 0);
    assert_eq!(probe(&mut host, 0, 22, 0), 3);
    assert_eq!(probe(&mut host, 0, 23, 0), 1f32.to_bits());
    assert_eq!(probe(&mut host, 0, 24, 0), (-1f32).to_bits());
    assert_eq!(probe(&mut host, 0, 25, 0), (-1f32).to_bits());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    println!(
        "actual {} MarkFragments uses published selected world geometry",
        if native { "owned-native" } else { "QVM" }
    );
}

pub(super) fn remaps(native: bool) {
    use qa_app::renderer::{
        AssetPreparer, SceneComposer,
        console::FontAssets,
        hud::{HudAssets, HudView},
    };
    use qa_compat::services::{ModelLoader, ShaderLighting};
    use qa_core::{
        configstrings::{ConfigLayout, ConfigStrings},
        events::TextStore,
    };
    use qa_render::{
        cpu::{CpuBackend, CpuLimits},
        shader::{reference::ShaderRef, registration::ShaderOwner},
    };

    struct Backend {
        cpu: CpuBackend,
        attempts: usize,
        reject: bool,
    }
    impl AssetPreparer for Backend {
        fn prepare_assets(
            &mut self,
            assets: &qa_render::Assets,
            limits: Option<qa_render::Limits>,
        ) -> Result<(), String> {
            self.attempts += 1;
            if std::mem::take(&mut self.reject) {
                return Err("reject remap admission".into());
            }
            self.cpu
                .prepare_assets(assets, limits)
                .map_err(str::to_owned)
        }
    }
    let mut fixture = model_support::Fixture::new(&[
        ("models/test.md3", model_support::md3_surfaces()),
        (
            "models/test.skin",
            b"body,textures/glow\nglow,textures/glow\n".to_vec(),
        ),
        (
            "textures/body.tga",
            model_support::tga(2, 2, &[[10, 80, 20, 255]; 4]),
        ),
        (
            "textures/glow.tga",
            model_support::tga(2, 2, &[[10, 80, 20, 255]; 4]),
        ),
        (
            "remap/b.tga",
            model_support::tga(2, 2, &[[90, 20, 10, 255]; 4]),
        ),
        (
            "remap/c.tga",
            model_support::tga(2, 2, &[[20, 30, 150, 255]; 4]),
        ),
        (
            "remap/d.tga",
            model_support::tga(2, 2, &[[60, 70, 80, 255]; 4]),
        ),
        (
            "gfx/2d/bigchars.tga",
            model_support::tga(128, 128, &vec![[255; 4]; 128 * 128]),
        ),
        (
            "scripts/remap.shader",
            br#"
            textures/glow { cull none { map textures/glow.tga rgbGen identity } }
            remap/b { cull none { map remap/b.tga rgbGen identity } }
            remap/c { cull none { map remap/c.tga rgbGen identity } }
            remap/d { cull none { map remap/d.tga rgbGen identity } }
        "#
            .to_vec(),
        ),
    ]);
    let (mut runtime, clients) = runtime(2);
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let mut initial = data();
    for (at, value) in [
        (16, b"textures/glow".as_slice()),
        (512, b"remap/b"),
        (544, b"1.25 trailing\xff"),
        (576, b"remap/c"),
        (608, b""),
        (624, b"remap/absent-a"),
        (656, b"remap/absent-b"),
        (944, b"remap/d"),
        (960, b"gfx/2d/bigchars"),
    ] {
        initial[at..at + value.len() + 1].fill(0);
        initial[at..at + value.len()].copy_from_slice(value);
    }
    // Remove the original suffix so source identity is exactly textures/glow.
    initial[16 + b"textures/glow".len()] = 0;
    let modules = [
        (false, Some(clients[0]), 0),
        (true, None, 0),
        (false, Some(clients[1]), 1),
    ];
    let requests = modules
        .iter()
        .enumerate()
        .map(|(index, &(ui, client, seat))| {
            let mut row_initial = initial;
            if index == 1 {
                row_initial[900..924].fill(0);
                row_initial[900..919].copy_from_slice(b"models/missing.skin");
            }
            let program = if native {
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                {
                    native_scene_data(ui, &row_initial)
                }
                #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
                {
                    qvm_scene_data(ui, &row_initial)
                }
            } else {
                qvm_scene_data(ui, &row_initial)
            };
            let mut row = request(
                &mut runtime,
                ModuleId(index as u16 + 1),
                client.unwrap_or(clients[0]),
                program,
            );
            row.entries = (0..29).collect();
            row.frame.client = client;
            row.context.console.seat = SeatId::new(seat).unwrap();
            row
        })
        .collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.local_clients[..2].copy_from_slice(&[Some(clients[0]), Some(clients[1])]);
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut owner = ModelAssets::new(
        &host.runtime.vfs,
        qa_render::Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let font = FontAssets::load(
        &host.runtime.vfs,
        &mut owner,
        RuleSetId::Quake3,
        profile.image_settings,
        qa_render::material::ShaderCompilePolicy::default(),
        None,
    )
    .unwrap();
    assert!(font.native);
    let hud = HudAssets::from_font(RuleSetId::Quake3, font, 320).unwrap();
    assert!(
        ModelLoader::register_shader(
            &mut owner,
            &host.runtime.vfs,
            b"textures/glow",
            RuleSetId::Quake3,
            true,
            ShaderLighting::TwoDimensional
        )
        .unwrap()
            > 0
    );
    let (mut scene, _) =
        SceneComposer::load(Default::default(), owner.assets_mut(), vec![]).unwrap();
    let staged = scene.stage_hud(hud, None).unwrap();
    let limits = staged.limits();
    staged.commit();
    let mut backend = Backend {
        cpu: CpuBackend::load_with_limits(
            320,
            200,
            owner.assets(),
            CpuLimits {
                scene: limits,
                ..Default::default()
            },
        )
        .unwrap(),
        attempts: 0,
        reject: false,
    };
    owner
        .publish_admission(backend.cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    host.runtime.model_assets = Some(owner);
    host.runtime.scene = Some(scene);
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    let mut handles = [0; 3];
    for (index, &(_, client, _)) in modules.iter().enumerate() {
        handles[index] =
            scene_words(&mut host, ModuleId(index as u16 + 1), client, 0, &[], None).unwrap();
        assert!(handles[index] > 0);
    }
    assert_eq!(
        handles[0], handles[1],
        "UI and cgame share logical registration"
    );
    let lifetime = host.runtime.server.clients[clients[0].0 as usize].entity;
    let context = host
        .native_shaders()
        .context(ShaderOwner::Client(clients[0], lifetime), profile, None)
        .unwrap();
    let source_ref = ShaderRef::Native(qa_render::shader::reference::NativeShaderRef {
        context,
        node: qa_render::shader::context::ShaderNodeId(handles[0] - 1),
    });
    let source_binding = source_ref.source(host.native_shaders()).unwrap();
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    scene_words(&mut host, ModuleId(1), Some(clients[0]), 2, &[], None).unwrap();
    scene_words(&mut host, ModuleId(1), Some(clients[0]), 3, &[], None).unwrap();
    for (index, &(_, client, _)) in modules.iter().enumerate() {
        scene_words(&mut host, ModuleId(index as u16 + 1), client, 4, &[], None).unwrap();
    }

    let prefix = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    backend.reject = true;
    assert_eq!(
        scene_words(
            &mut host,
            ModuleId(2),
            None,
            26,
            &[944, 656, 2_147_483_647],
            Some(&mut backend)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        prefix
    );
    assert_eq!(
        source_ref
            .resolve(
                host.runtime.model_assets.as_ref().unwrap().assets(),
                host.native_shaders()
            )
            .unwrap()
            .binding(),
        source_binding
    );
    // Retry admits the source before discovering the missing target; its prefix remains.
    assert_eq!(
        scene_words(
            &mut host,
            ModuleId(2),
            None,
            26,
            &[944, 656, 2_147_483_647],
            Some(&mut backend)
        )
        .unwrap(),
        0
    );
    let catalog = host.runtime.model_assets.as_ref().unwrap().catalog();
    assert!(
        catalog
            .shader_outcome(b"remap/d", profile, ShaderLighting::World(None), None)
            .unwrap()
            .defaulted
            .is_none()
    );
    assert!(
        catalog
            .shader_outcome(
                b"remap/absent-b",
                profile,
                ShaderLighting::World(None),
                None
            )
            .unwrap()
            .defaulted
            .is_some()
    );
    assert_eq!(
        scene_words(
            &mut host,
            ModuleId(2),
            None,
            26,
            &[624, 2_147_483_647, 2_147_483_647],
            Some(&mut backend)
        )
        .unwrap(),
        0
    );
    assert!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .catalog()
            .shader_outcome(
                b"remap/absent-a",
                profile,
                ShaderLighting::World(None),
                None
            )
            .unwrap()
            .defaulted
            .is_some()
    );
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);

    assert_eq!(
        scene_words(
            &mut host,
            ModuleId(2),
            None,
            26,
            &[16, 512, 544],
            Some(&mut backend)
        )
        .unwrap(),
        0
    );
    let selected = |host: &FrameHost| {
        source_ref
            .resolve(
                host.runtime.model_assets.as_ref().unwrap().assets(),
                host.native_shaders(),
            )
            .unwrap()
    };
    assert_eq!(selected(&host).time_offset(), 1.25);
    let attempts = backend.attempts;
    host.runtime.vfs.take_lookup_count();
    assert_eq!(
        scene_words(&mut host, ModuleId(2), None, 26, &[16, 512, 0], None).unwrap(),
        0
    );
    assert_eq!(selected(&host).time_offset(), 1.25);
    assert_eq!(
        scene_words(&mut host, ModuleId(2), None, 26, &[16, 512, 608], None).unwrap(),
        0
    );
    assert_eq!(selected(&host).time_offset(), 0.0);
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    assert_eq!(backend.attempts, attempts);

    let b = scene_words(&mut host, ModuleId(2), None, 27, &[512], Some(&mut backend)).unwrap();
    assert!(b > 0);
    scene_words(&mut host, ModuleId(2), None, 28, &[u64::from(b)], None).unwrap();
    scene_words(&mut host, ModuleId(2), None, 26, &[16, 512, 544], None).unwrap();
    let b_binding = selected(&host).binding();
    scene_words(
        &mut host,
        ModuleId(2),
        None,
        26,
        &[512, 576, 608],
        Some(&mut backend),
    )
    .unwrap();
    assert_eq!(
        (selected(&host).binding(), selected(&host).time_offset()),
        (b_binding, 1.25),
        "one hop keeps A drawing B after B remaps C"
    );
    scene_words(&mut host, ModuleId(2), None, 4, &[], None).unwrap();

    let values = qa_ui::values::ValueLayout::load(&[]).unwrap();
    let mut hud_state = HudState::with_capacity(0, 0, 0, values.capacity());
    let mut texts = TextStore::load(1, 8).unwrap();
    hud_state.notify[0] = Some(HudLine {
        text: texts.insert(b"A").unwrap(),
        started_at: 0.,
        until: 2.,
    });
    let layout = ConfigLayout::QUAKE3;
    let configs =
        ConfigStrings::load((0..layout.count).map(|i| layout.capacity(i).unwrap())).unwrap();
    let localization = qa_content::localization::Catalog::default();
    let mut scene = host.runtime.scene.take().unwrap();
    let assets = host.runtime.model_assets.as_ref().unwrap().assets();
    let mut font_front = qa_render::FrontEnd::load(Default::default()).unwrap();
    let mut font_frame = font_front.begin_frame([3, 5, 7, 255]).unwrap();
    let hud_stats = scene.append_hud(
        &mut font_frame,
        assets,
        HudView {
            seat: None,
            file_source: None,
            hud: &hud_state,
            texts: &texts,
            configs: &configs,
            layout,
            values: &values,
            localization: &localization,
            rules: RuleSetId::Quake3,
            time: EventTime(0),
            style_time: ThinkTime::Milliseconds(0),
            server_frame: 0,
            client: clients[0],
            client_lifetime: lifetime,
            style_interpolation: 0,
            show_text: true,
        },
        qa_render::Refdef {
            viewport: qa_render::Viewport {
                width: 160,
                height: 200,
                ..Default::default()
            },
            ..Default::default()
        },
        host.native_shaders(),
        ShaderOwner::Client(clients[0], lifetime),
    );
    assert_eq!((hud_stats.glyphs, hud_stats.rejected), (1, 0));
    host.runtime.scene = Some(scene);
    scene_words(&mut host, ModuleId(2), None, 26, &[960, 512, 0], None).unwrap();
    let assets = host.runtime.model_assets.as_ref().unwrap().assets();
    let mut font_packet = font_frame.finish();
    let font_shader = font_packet
        .commands()
        .iter()
        .find_map(|command| match command {
            Command::Draw2d(draw) => Some(draw.shader),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        font_shader
            .resolve(assets, host.native_shaders())
            .unwrap()
            .binding(),
        b_binding,
        "queued font follows the final native remap"
    );
    let mut packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    let native_draws: Vec<_> = packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::Draw2d(draw) => Some(draw.shader),
            _ => None,
        })
        .collect();
    assert_eq!(native_draws.len(), 4);
    assert_eq!(
        native_draws[0].source(host.native_shaders()),
        Some(source_binding)
    );
    assert_eq!(
        native_draws[0]
            .resolve(assets, host.native_shaders())
            .unwrap()
            .binding(),
        b_binding
    );
    assert_eq!(
        native_draws[1]
            .resolve(assets, host.native_shaders())
            .unwrap()
            .binding(),
        b_binding
    );
    assert_eq!(
        native_draws[2]
            .resolve(assets, host.native_shaders())
            .unwrap()
            .binding(),
        source_binding,
        "peer context remains unchanged"
    );
    assert_ne!(
        native_draws[3]
            .resolve(assets, host.native_shaders())
            .unwrap()
            .binding(),
        b_binding
    );
    let view = packet
        .commands()
        .iter()
        .find_map(|command| match command {
            Command::View(view) if view.scene.polys.count != 0 => Some(view),
            _ => None,
        })
        .unwrap();
    assert_eq!(packet.draws(view.scene.draws).len(), 2);
    assert!(packet.draws(view.scene.draws).iter().all(|draw| {
        draw.shader
            .unwrap()
            .resolve(assets, host.native_shaders())
            .unwrap()
            .binding()
            == b_binding
    }));
    packet.resolve_shaders(assets, host.native_shaders());
    assert_eq!(backend.cpu.render(&packet, assets).rejected, 0);
    assert_eq!(
        backend.cpu.pixels()[10 * 320 + 30],
        u32::from_le_bytes([20, 30, 150, 255])
    );
    assert_eq!(
        backend.cpu.pixels()[10 * 320 + 190],
        u32::from_le_bytes([10, 80, 20, 255])
    );
    assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
    font_packet.resolve_shaders(assets, host.native_shaders());
    assert_eq!(backend.cpu.render(&font_packet, assets).rejected, 0);
    assert!(font_front.recycle(font_packet).is_ok());

    // A later model shader variant starts without the old name's remap pointer.
    let model = scene_words(
        &mut host,
        ModuleId(1),
        Some(clients[0]),
        12,
        &[],
        Some(&mut backend),
    )
    .unwrap();
    assert!(model > 0);
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let registrations = owner
        .catalog()
        .renderer_registrations(
            qa_compat::services::RendererRegistration::Model,
            model as i32,
            profile,
        )
        .unwrap();
    let resource = owner
        .catalog()
        .renderer_registration(model as i32, profile)
        .unwrap();
    let selected = owner
        .catalog()
        .bindings(resource, 0, Default::default())
        .unwrap()
        .choices()
        .next()
        .unwrap();
    let nodes = host
        .native_shaders()
        .resource_nodes(context, selected.resource)
        .unwrap();
    let glow_key = owner.assets().find_shader_name(b"textures/glow").unwrap();
    let node = registrations
        .iter()
        .zip(nodes)
        .find(|(row, _)| row.key == glow_key)
        .unwrap()
        .1;
    let fresh = ShaderRef::Native(qa_render::shader::reference::NativeShaderRef {
        context,
        node: *node,
    });
    assert_ne!(fresh, source_ref);
    assert_eq!(
        fresh
            .resolve(owner.assets(), host.native_shaders())
            .unwrap()
            .binding(),
        fresh.source(host.native_shaders()).unwrap()
    );
    // Physical readiness in a peer does not replace first logical registration in this client.
    let peer_skin = scene_words(
        &mut host,
        ModuleId(3),
        Some(clients[1]),
        16,
        &[],
        Some(&mut backend),
    )
    .unwrap();
    assert!(peer_skin > 0);
    let peer_d = scene_words(
        &mut host,
        ModuleId(3),
        Some(clients[1]),
        27,
        &[944],
        Some(&mut backend),
    )
    .unwrap();
    assert!(peer_d > 0);
    let stable_prefix = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    let stable_attempts = backend.attempts;
    let c_binding = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .catalog()
        .shader_outcome(b"remap/c", profile, ShaderLighting::World(None), None)
        .unwrap()
        .shader;
    for (module, client, command, args, expected, missing) in [
        (ModuleId(1), Some(clients[0]), 16, &[][..], peer_skin, false),
        (ModuleId(2), None, 16, &[][..], 0, true),
        (ModuleId(1), Some(clients[0]), 27, &[944][..], peer_d, false),
    ] {
        if command == 27 {
            let smp = host
                .console
                .cvars
                .bind(
                    "r_smp",
                    Context {
                        source: RuleSetId::Quake3,
                        ..Context::default()
                    },
                )
                .unwrap();
            host.console.cvars.set(smp.canonical(), 1.).unwrap();
            assert_eq!(
                host.console
                    .cvars
                    .integer_in(smp.canonical(), RuleSetId::Quake3),
                1
            );
        }
        let mut shader_handle = None;
        for first in [true, false] {
            host.runtime.vfs.take_lookup_count();
            assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
            scene_words(&mut host, ModuleId(2), None, 26, &[16, 512, 0], None).unwrap();
            scene_words(&mut host, ModuleId(1), Some(clients[0]), 4, &[], None).unwrap();
            let returned = scene_words(&mut host, module, client, command, args, None).unwrap();
            if command == 27 {
                if first {
                    assert!(returned > 0);
                    shader_handle = Some(returned);
                } else {
                    assert_eq!(Some(returned), shader_handle);
                }
            } else {
                assert_eq!(returned, expected);
            }
            scene_words(&mut host, ModuleId(2), None, 26, &[16, 576, 0], None).unwrap();
            scene_words(&mut host, ModuleId(1), Some(clients[0]), 4, &[], None).unwrap();
            let mut packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
            let assets = host.runtime.model_assets.as_ref().unwrap().assets();
            let pictures: Vec<_> = packet
                .commands()
                .iter()
                .filter_map(|command| match command {
                    Command::Draw2d(draw) => Some(draw.shader),
                    _ => None,
                })
                .collect();
            assert_eq!(pictures.len(), 2);
            assert_eq!(
                pictures[0]
                    .resolve(assets, host.native_shaders())
                    .unwrap()
                    .binding(),
                if first { b_binding } else { c_binding },
                "first logical registration seals queued draws; a cached repeat leaves them live"
            );
            assert_eq!(
                pictures[1]
                    .resolve(assets, host.native_shaders())
                    .unwrap()
                    .binding(),
                c_binding
            );
            packet.resolve_shaders(assets, host.native_shaders());
            assert_eq!(backend.cpu.render(&packet, assets).rejected, 0);
            assert_eq!(
                backend.cpu.pixels()[10 * 320 + 30],
                u32::from_le_bytes([20, 30, 150, 255])
            );
            assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
            assert_eq!(
                host.runtime
                    .model_assets
                    .as_ref()
                    .unwrap()
                    .assets()
                    .counts(),
                stable_prefix
            );
            assert_eq!(backend.attempts, stable_attempts);
            let lookups = host.runtime.vfs.take_lookup_count();
            if missing && first {
                assert!(lookups > 0);
            } else {
                assert_eq!(lookups, 0);
            }
        }
    }
    // Previously defaulted names are usable on later lookup, so these bad operands are now reached.
    assert!(
        scene_words(
            &mut host,
            ModuleId(2),
            None,
            26,
            &[624, 2_147_483_647, 0],
            None
        )
        .is_err()
    );
    assert!(
        scene_words(
            &mut host,
            ModuleId(1),
            Some(clients[0]),
            26,
            &[16, 656, 2_147_483_647],
            None
        )
        .is_err()
    );
    assert!(scene_words(&mut host, ModuleId(3), Some(clients[1]), 0, &[], None).unwrap() > 0);
    assert_eq!(host.module_counts(ModuleId(3)).unwrap().traps, 0);
}
