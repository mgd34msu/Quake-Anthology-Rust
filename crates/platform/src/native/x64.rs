//! Hardware ABI gates used only by the owned Linux child. Guest instructions
//! and stack bytes never execute in, or borrow a stack from, the engine.
use super::import;

unsafe extern "sysv64" {
    #[link_name = "qa_native_x64_call"]
    pub(super) fn call(
        entry: u64,
        abi: u64,
        arguments: *const [u64; 13],
        top: u64,
        floats: *const [u64; 8],
        control: u64,
    ) -> u64;
    #[link_name = "qa_native_x64_system_v_import"]
    pub(super) fn system_v_import();
    #[link_name = "qa_native_x64_microsoft_import"]
    pub(super) fn microsoft_import();
    #[link_name = "qa_native_x64_system_v_function"]
    pub(super) fn system_v_function();
    #[link_name = "qa_native_x64_microsoft_function"]
    pub(super) fn microsoft_function();
}

// The controller frame is private. Import capture contains raw guest words,
// never a Rust reference to guest storage. Before Rust handles a packet, the
// gate saves guest state, restores controller floating-point state and switches
// back to that private stack. The controller may then publish every shared byte
// while the parent enforces a kernel stop. Only one foreign call is active in
// this single-thread child; nested exports are not admitted by this gate yet.
std::arch::global_asm!(
    r#"
    .pushsection .bss
    .balign 16
.Lcontroller_stack: .zero 8
.Lguest_bottom: .zero 8
.Lguest_top: .zero 8
    .balign 16
    // rsp, flags, rbx, rbp, rdi, rsi, r12-r15; 14 ABI words; FXSAVE.
.Limport_state: .zero 720
    .popsection

    .pushsection .text
    .global {call}
    .hidden {call}
{call}:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    sub rsp, 536
    fxsave64 [rsp]
    mov [rsp + 512], r9
    mov r12, r8
    mov [rip + .Lcontroller_stack], rsp
    mov [rip + .Lguest_top], rcx
    mov rax, rcx
    sub rax, {stack_bytes}
    mov [rip + .Lguest_bottom], rax
    mov r11, rdi
    mov r10, rdx
    movq xmm0, [r12]
    movq xmm1, [r12 + 8]
    movq xmm2, [r12 + 16]
    movq xmm3, [r12 + 24]
    movq xmm4, [r12 + 32]
    movq xmm5, [r12 + 40]
    movq xmm6, [r12 + 48]
    movq xmm7, [r12 + 56]
    mov rsp, rcx
    test rsi, rsi
    jne .Lmicrosoft_call
    // Seven stack words. RSP is 16-byte aligned before CALL.
    sub rsp, 64
    lea rsi, [r10 + 48]
    mov rdi, rsp
    mov ecx, 7
    cld
    rep movsq
    mov rdi, [r10]
    mov rsi, [r10 + 8]
    mov rdx, [r10 + 16]
    mov rcx, [r10 + 24]
    mov r8, [r10 + 32]
    mov r9, [r10 + 40]
    jmp .Linvoke
.Lmicrosoft_call:
    // 32-byte shadow area followed by nine stack words.
    sub rsp, 112
    lea rsi, [r10 + 32]
    lea rdi, [rsp + 32]
    mov ecx, 9
    cld
    rep movsq
    mov rcx, [r10]
    mov rdx, [r10 + 8]
    mov r8, [r10 + 16]
    mov r9, [r10 + 24]
.Linvoke:
    mov rax, [rip + .Lcontroller_stack]
    movzx eax, byte ptr [rax + 513]
    call r11
    cld
    mov rsp, [rip + .Lcontroller_stack]
    cmp byte ptr [rsp + 512], 0
    je .Lresult
    cmp byte ptr [rsp + 512], 1
    je .Lfloat_result
    cmp byte ptr [rsp + 512], 2
    jne .Lvoid_result
    movq rax, xmm0
    jmp .Lresult
.Lfloat_result:
    movd eax, xmm0
    jmp .Lresult
.Lvoid_result:
    xor eax, eax
.Lresult:
    fxrstor64 [rsp]
    add rsp, 536
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret

    .global {system_v_import}
    .hidden {system_v_import}
{system_v_import}:
    mov qword ptr [rip + .Limport_state + 704], 0
    mov [rip + .Limport_state + 80], rdi
    mov [rip + .Limport_state + 88], rsi
    mov [rip + .Limport_state + 96], rdx
    mov [rip + .Limport_state + 104], rcx
    mov [rip + .Limport_state + 112], r8
    mov [rip + .Limport_state + 120], r9
    lea r10, [rsp + 8]
    mov r11d, 6
    jmp .Limport
    .global {microsoft_import}
    .hidden {microsoft_import}
{microsoft_import}:
    mov qword ptr [rip + .Limport_state + 704], 0
    mov [rip + .Limport_state + 80], rcx
    mov [rip + .Limport_state + 88], rdx
    mov [rip + .Limport_state + 96], r8
    mov [rip + .Limport_state + 104], r9
    lea r10, [rsp + 40]
    mov r11d, 4
    jmp .Limport
    .global {system_v_function}
    .hidden {system_v_function}
{system_v_function}:
    mov qword ptr [rip + .Limport_state + 704], 1
    mov [rip + .Limport_state + 80], r11
    mov [rip + .Limport_state + 88], rdi
    mov [rip + .Limport_state + 96], rsi
    mov [rip + .Limport_state + 104], rdx
    mov [rip + .Limport_state + 112], rcx
    mov [rip + .Limport_state + 120], r8
    mov [rip + .Limport_state + 128], r9
    lea r10, [rsp + 8]
    mov r11d, 7
    jmp .Limport
    .global {microsoft_function}
    .hidden {microsoft_function}
{microsoft_function}:
    mov qword ptr [rip + .Limport_state + 704], 1
    mov [rip + .Limport_state + 80], r11
    mov [rip + .Limport_state + 88], rcx
    mov [rip + .Limport_state + 96], rdx
    mov [rip + .Limport_state + 104], r8
    mov [rip + .Limport_state + 112], r9
    lea r10, [rsp + 40]
    mov r11d, 5
.Limport:
    mov [rip + .Limport_state], rsp
    pushfq
    pop qword ptr [rip + .Limport_state + 8]
    mov [rip + .Limport_state + 16], rbx
    mov [rip + .Limport_state + 24], rbp
    mov [rip + .Limport_state + 32], rdi
    mov [rip + .Limport_state + 40], rsi
    mov [rip + .Limport_state + 48], r12
    mov [rip + .Limport_state + 56], r13
    mov [rip + .Limport_state + 64], r14
    mov [rip + .Limport_state + 72], r15
    fxsave64 [rip + .Limport_state + 192]
    // Only raw assembly loads touch the guest stack. Missing variadic words
    // are not read as Rust values or used by a syscall without its ABI shape.
    cmp rsp, [rip + .Lguest_bottom]
    jb .Linvalid
    // Check the exact end: source + (14 - first_word) * 8.
    mov rax, 14
    sub rax, r11
    lea rax, [r10 + rax * 8]
    cmp rax, [rip + .Lguest_top]
    ja .Linvalid
    lea rax, [rip + .Limport_state + 80]
.Lcopy_import:
    mov rdx, [r10]
    mov [rax + r11 * 8], rdx
    add r10, 8
    inc r11
    cmp r11, 14
    jb .Lcopy_import
    cld
    mov rsp, [rip + .Lcontroller_stack]
    fxrstor64 [rsp]
    lea rdi, [rip + .Limport_state + 80]
    lea rsi, [rip + .Limport_state + 352]
    mov rdx, [rip + .Limport_state + 704]
    call {import}
    fxrstor64 [rip + .Limport_state + 192]
    cmp edx, 1
    je .Limport_float
    cmp edx, 2
    je .Limport_double
    cmp edx, 3
    jne .Limport_result
    xor eax, eax
    jmp .Limport_result
.Limport_float:
    movd xmm0, eax
    jmp .Limport_result
.Limport_double:
    movq xmm0, rax
.Limport_result:
    mov rbx, [rip + .Limport_state + 16]
    mov rbp, [rip + .Limport_state + 24]
    mov rdi, [rip + .Limport_state + 32]
    mov rsi, [rip + .Limport_state + 40]
    mov r12, [rip + .Limport_state + 48]
    mov r13, [rip + .Limport_state + 56]
    mov r14, [rip + .Limport_state + 64]
    mov r15, [rip + .Limport_state + 72]
    push qword ptr [rip + .Limport_state + 8]
    popfq
    mov rsp, [rip + .Limport_state]
    ret
.Linvalid:
    mov eax, 231
    mov edi, 125
    syscall
    ud2
    .popsection
"#,
    call = sym call,
    system_v_import = sym system_v_import,
    microsoft_import = sym microsoft_import,
    system_v_function = sym system_v_function,
    microsoft_function = sym microsoft_function,
    import = sym import,
    stack_bytes = const super::STACK,
);
