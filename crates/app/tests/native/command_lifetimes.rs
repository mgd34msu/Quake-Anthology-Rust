//! Reuse the same session assertions with actual ELF/PE child syscalls.
use super::*;
#[path = "../support/service_program.rs"]
#[allow(dead_code)]
mod service_program;
#[path = "../modules/command_lifetimes.rs"]
#[allow(dead_code)]
mod shared;

fn program(encoding: Encoding, text: &[u8]) -> shared::ProducerProgram {
    let (command, reserve, saved, text_register, mode, number) = match encoding {
        Encoding::Elf => (0xff, 24, 8, 0x15, 0xbe, 0xbf),
        Encoding::Pe => (0xf9, 56, 40, 0x05, 0xba, 0xb9),
    };
    let mut code = vec![0x83, command, 0, 0x74, 0];
    code.extend([0x48, 0x83, 0xec, reserve]);
    code.extend([
        0x89,
        if encoding == Encoding::Elf {
            0x7c
        } else {
            0x4c
        },
        0x24,
        saved,
    ]);
    code.extend([0x48, 0x8b, 0x05]);
    code.extend((0x700i32 - (code.len() as i32 + 4)).to_le_bytes());
    // SystemV uses rdx for the text, Windows x64 r8; both are the third
    // argument after syscall ordinal14 and EXEC_APPEND2.
    code.extend([
        if encoding == Encoding::Elf {
            0x48
        } else {
            0x4c
        },
        0x8d,
        text_register,
    ]);
    code.extend((0x200i32 - (code.len() as i32 + 4)).to_le_bytes());
    code.push(mode);
    code.extend(2u32.to_le_bytes());
    code.push(number);
    code.extend(14u32.to_le_bytes());
    code.extend([0xff, 0xd0, 0x83, 0x7c, 0x24, saved, 2, 0x75, 2, 0x0f, 0x0b]);
    code.extend([0x48, 0x83, 0xc4, reserve]);
    code[4] = (code.len() - 5) as u8;
    code.extend([0x31, 0xc0, 0xc3]);
    assert!(code.len() <= 64);
    assert!(text.len() < 0x400);
    let image = function_image(encoding, &code);
    let name = if encoding == Encoding::Elf {
        b"vmMain".as_slice()
    } else {
        b"GetGameAPI".as_slice()
    };
    let entry = (image.symbol(name).unwrap().address - image.base) as usize;
    let mut exports: Vec<_> = (0..3)
        .map(|command| NamedExport {
            name,
            command: Some(command),
            parameters: &[NativeScalar::Word; 13],
            result: NativeScalar::Word,
        })
        .collect();
    exports.push(NamedExport {
        name: b"dllEntry",
        command: None,
        parameters: &[NativeScalar::Word],
        result: NativeScalar::Void,
    });
    let mut vm = Vm::map_image(image, &exports, &[], Duration::from_secs(3)).unwrap();
    let callback = vm.import_callback();
    vm.process.memory_mut().unwrap()[entry + 0x200..entry + 0x200 + text.len()]
        .copy_from_slice(text);
    vm.process.memory_mut().unwrap()[entry + 0x200 + text.len()] = 0;
    (
        Program::Native {
            vm: Box::new(vm),
            imports: &Q3_SERVER,
        },
        vec![Export::words(CallbackId(3), &[callback])],
    )
}
pub(super) fn run() {
    for encoding in [Encoding::Elf, Encoding::Pe] {
        let make = |text: &[u8]| program(encoding, text);
        shared::shutdown_retires_alias_wait_and_shutdown_output(make);
        shared::failed_initialization_cannot_leave_queued_work(make);
        shared::stale_generation_and_new_publication_use_exact_lifetimes(make);
        shared::absent_cold_row_cannot_lend_its_live_anchor(make);
        shared::transferred_owner_invalidates_wait_before_healthy_work(make);
        shared::fatal_export_retires_before_its_queued_text(make);
        shared::queued_binding_keeps_alias_producer_and_retires_with_it(make);
        shared::manual_hold_retirement_preserves_human_press(make);
        shared::purged_keyup_cannot_leave_module_button_held(make, true);
        shared::stale_empty_buffer_retires_holds_before_the_next_frame(make);
        println!("native command producer lifetimes passed: {encoding:?}");
    }
}
