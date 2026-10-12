//! i386 System V cdecl hardware gates in the same owned native child backend.
//! Only stack capture and hardware ABI differ from the x86-64 gate.
use super::import32;

unsafe extern "C" {
    #[link_name = "qa_native_x86_call"]
    pub(super) fn call(
        entry: u64,
        abi: u64,
        arguments: *const [u64; 13],
        top: u64,
        floats: *const [u64; 8],
        control: u64,
    ) -> u64;
    #[link_name = "qa_native_x86_import"]
    pub(super) fn system_v_import();
    #[link_name = "qa_native_x86_function"]
    pub(super) fn system_v_function();
    #[link_name = "qa_native_x86_callback_top"]
    pub(super) fn callback_top() -> u64;
}
// PE32 calling conventions are not admitted. These slots remain zero in the
// READY envelope rather than pretending cdecl implements stdcall or thiscall.
pub(super) const MICROSOFT_IMPORT: usize = 0;
pub(super) const MICROSOFT_FUNCTION: usize = 0;

// The private controller frame preserves the outer gate and import capture
// across nested native callbacks. PIC addresses use a local call/pop base;
// no executable text relocation or foreign reference to Rust stacks is needed.
std::arch::global_asm!(
    r#"
    .pushsection .bss
    .balign 16
.Lx86_state:
    .zero 16
    // guest esp/flags/ebx/esi/edi/ebp, kind/pc, result, words, FXSAVE.
.Lx86_import: .zero 672
    .popsection
    .pushsection .text
    .global {call}
    .hidden {call}
{call}:
    push ebp
    mov ebp, esp
    push ebx
    push esi
    push edi
    and esp, -16
    sub esp, 1248
    fxsave [esp]
    mov [esp + 548], ebp
    mov eax, [ebp + 8]
    mov [esp + 528], eax
    mov eax, [ebp + 24]
    mov [esp + 532], eax
    mov eax, [ebp + 28]
    mov [esp + 524], eax
    mov eax, [ebp + 40]
    mov [esp + 520], eax
    call .Lx86_call_pc
.Lx86_call_pc:
    pop ebx
    .set .Lx86_call_delta, .Lx86_state - .Lx86_call_pc
    add ebx, offset .Lx86_call_delta
    mov eax, [ebx]
    mov [esp + 536], eax
    mov eax, [ebx + 4]
    mov [esp + 540], eax
    mov eax, [ebx + 8]
    mov [esp + 544], eax
    lea esi, [ebx + 16]
    lea edi, [esp + 560]
    mov ecx, 168
    cld
    rep movsd
    mov [ebx], esp
    mov eax, [esp + 524]
    mov [ebx + 8], eax
    cmp dword ptr [ebx + 4], 0
    jne .Lx86_bound
    sub eax, {stack_bytes}
    mov [ebx + 4], eax
.Lx86_bound:
    mov eax, [esp + 528]
    mov esi, [esp + 532]
    mov edx, [esp + 524]
    mov esp, edx
    sub esp, 112
    mov edi, esp
    mov ecx, 26
    rep movsd
    call eax
    cld
    mov esp, [ebx]
    cmp byte ptr [esp + 520], 0
    je .Lx86_integer_result
    cmp byte ptr [esp + 520], 1
    je .Lx86_float_result
    cmp byte ptr [esp + 520], 2
    jne .Lx86_void_result
    fstp qword ptr [esp + 512]
    jmp .Lx86_restore
.Lx86_float_result:
    fstp dword ptr [esp + 512]
    mov dword ptr [esp + 516], 0
    jmp .Lx86_restore
.Lx86_void_result:
    xor eax, eax
.Lx86_integer_result:
    mov [esp + 512], eax
    mov dword ptr [esp + 516], 0
.Lx86_restore:
    mov eax, [esp + 536]
    mov [ebx], eax
    mov eax, [esp + 540]
    mov [ebx + 4], eax
    mov eax, [esp + 544]
    mov [ebx + 8], eax
    lea esi, [esp + 560]
    lea edi, [ebx + 16]
    mov ecx, 168
    rep movsd
    mov eax, [esp + 512]
    mov edx, [esp + 516]
    mov ebp, [esp + 548]
    fxrstor [esp]
    lea esp, [ebp - 12]
    pop edi
    pop esi
    pop ebx
    pop ebp
    ret

    .global {callback_top}
    .hidden {callback_top}
{callback_top}:
    call .Lx86_top_pc
.Lx86_top_pc:
    pop eax
    .set .Lx86_top_delta, .Lx86_state - .Lx86_top_pc
    add eax, offset .Lx86_top_delta
    mov eax, [eax + 16]
    sub eax, 16
    and eax, -16
    xor edx, edx
    ret

    .global {system_v_import}
    .hidden {system_v_import}
{system_v_import}:
    push 0
    jmp .Lx86_capture
    .global {system_v_function}
    .hidden {system_v_function}
{system_v_function}:
    push 1
.Lx86_capture:
    pushfd
    push eax
    push ebx
    push esi
    push edi
    push ebp
    call .Lx86_import_pc
.Lx86_import_pc:
    pop ebx
    .set .Lx86_import_delta, .Lx86_state - .Lx86_import_pc
    add ebx, offset .Lx86_import_delta
    lea edx, [esp + 28]
    cmp esp, [ebx + 4]
    jb .Lx86_invalid
    lea ecx, [edx + 108]
    cmp ecx, [ebx + 8]
    ja .Lx86_invalid
    mov [ebx + 16], edx
    mov eax, [esp + 20]
    mov [ebx + 20], eax
    mov eax, [esp + 12]
    mov [ebx + 24], eax
    mov eax, [esp + 8]
    mov [ebx + 28], eax
    mov eax, [esp + 4]
    mov [ebx + 32], eax
    mov eax, [esp]
    mov [ebx + 36], eax
    mov eax, [esp + 24]
    mov [ebx + 40], eax
    mov eax, [edx]
    mov [ebx + 44], eax
    fxsave [ebx + 176]
    lea esi, [edx + 4]
    lea edi, [ebx + 64]
    cmp dword ptr [ebx + 40], 0
    je .Lx86_raw_words
    mov eax, [esp + 16]
    mov [edi], eax
    mov dword ptr [edi + 4], 0
    add edi, 8
    mov ecx, 26
    cld
    rep movsd
    jmp .Lx86_controller_import
.Lx86_raw_words:
    mov ecx, 14
    cld
.Lx86_raw_copy:
    lodsd
    stosd
    mov dword ptr [edi], 0
    add edi, 4
    loop .Lx86_raw_copy
.Lx86_controller_import:
    mov esp, [ebx]
    fxrstor [esp]
    // Keep ESP aligned before CALL. This helper returns through an explicit
    // out pointer, never i386's compiler-dependent hidden sret convention.
    sub esp, 32
    lea eax, [ebx + 48]
    mov [esp], eax
    lea eax, [ebx + 64]
    mov [esp + 4], eax
    mov eax, [ebx + 40]
    mov [esp + 8], eax
    mov eax, [ebx + 44]
    mov [esp + 12], eax
    mov eax, [ebx + 16]
    mov [esp + 16], eax
    call {import}
    add esp, 32
    cmp dword ptr [ebx + 56], 4
    je .Lx86_trapped
    fxrstor [ebx + 176]
    cmp dword ptr [ebx + 56], 1
    je .Lx86_import_float
    cmp dword ptr [ebx + 56], 2
    je .Lx86_import_double
    cmp dword ptr [ebx + 56], 3
    je .Lx86_import_void
    mov eax, [ebx + 48]
    jmp .Lx86_import_return
.Lx86_import_float:
    fld dword ptr [ebx + 48]
    jmp .Lx86_import_return
.Lx86_import_double:
    fld qword ptr [ebx + 48]
    jmp .Lx86_import_return
.Lx86_import_void:
    xor eax, eax
.Lx86_import_return:
    mov edx, [ebx + 16]
    push dword ptr [ebx + 20]
    popfd
    mov ebp, [ebx + 36]
    mov edi, [ebx + 32]
    mov esi, [ebx + 28]
    mov ebx, [ebx + 24]
    mov esp, edx
    ret
.Lx86_trapped:
    mov esp, [ebx]
    mov dword ptr [esp + 512], 0
    mov dword ptr [esp + 516], 0
    jmp .Lx86_restore
.Lx86_invalid:
    mov eax, 252
    mov ebx, 125
    int 0x80
    ud2
    .popsection
"#,
    call = sym call,
    system_v_import = sym system_v_import,
    system_v_function = sym system_v_function,
    callback_top = sym callback_top,
    import = sym import32,
    stack_bytes = const super::STACK,
);
