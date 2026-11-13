// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2020, tevador <tevador@gmail.com>
// Copyright (c) 2019-2021, SChernykh <https://github.com/SChernykh>
// Copyright (c) 2019-2021, XMRig <https://github.com/xmrig>
// Copyright (c) 2026 RxMiner contributors
// Rust-integrated assembler templates, ported from the attributed RandomX
// templates. No C/C++ compiler, external assembler or native library is used.
// See ../randomx_core/LICENSE for the BSD 3-Clause terms.
use core::arch::global_asm;

global_asm!(
    r#"
.if {macho}
.section __TEXT,__const
.elseif {elf}
.section .rodata
.else
.section .rdata,"dr"
.endif
.global rxrust_randomx_program_prologue
.if {elf}
.hidden rxrust_randomx_program_prologue
.endif
.if {macho}
.global _rxrust_randomx_program_prologue
.set _rxrust_randomx_program_prologue, rxrust_randomx_program_prologue
.endif
.global rxrust_randomx_program_loop_begin
.if {elf}
.hidden rxrust_randomx_program_loop_begin
.endif
.if {macho}
.global _rxrust_randomx_program_loop_begin
.alt_entry _rxrust_randomx_program_loop_begin
.alt_entry rxrust_randomx_program_loop_begin
.set _rxrust_randomx_program_loop_begin, rxrust_randomx_program_loop_begin
.endif
.global rxrust_randomx_program_loop_load
.if {elf}
.hidden rxrust_randomx_program_loop_load
.endif
.if {macho}
.global _rxrust_randomx_program_loop_load
.alt_entry _rxrust_randomx_program_loop_load
.alt_entry rxrust_randomx_program_loop_load
.set _rxrust_randomx_program_loop_load, rxrust_randomx_program_loop_load
.endif
.global rxrust_randomx_program_start
.if {elf}
.hidden rxrust_randomx_program_start
.endif
.if {macho}
.global _rxrust_randomx_program_start
.alt_entry _rxrust_randomx_program_start
.alt_entry rxrust_randomx_program_start
.set _rxrust_randomx_program_start, rxrust_randomx_program_start
.endif
.global rxrust_randomx_program_read_dataset
.if {elf}
.hidden rxrust_randomx_program_read_dataset
.endif
.if {macho}
.global _rxrust_randomx_program_read_dataset
.alt_entry _rxrust_randomx_program_read_dataset
.alt_entry rxrust_randomx_program_read_dataset
.set _rxrust_randomx_program_read_dataset, rxrust_randomx_program_read_dataset
.endif
.global rxrust_randomx_program_read_dataset_v2
.if {elf}
.hidden rxrust_randomx_program_read_dataset_v2
.endif
.if {macho}
.global _rxrust_randomx_program_read_dataset_v2
.alt_entry _rxrust_randomx_program_read_dataset_v2
.alt_entry rxrust_randomx_program_read_dataset_v2
.set _rxrust_randomx_program_read_dataset_v2, rxrust_randomx_program_read_dataset_v2
.endif
.global rxrust_randomx_program_read_dataset_sshash_init
.if {elf}
.hidden rxrust_randomx_program_read_dataset_sshash_init
.endif
.if {macho}
.global _rxrust_randomx_program_read_dataset_sshash_init
.alt_entry _rxrust_randomx_program_read_dataset_sshash_init
.alt_entry rxrust_randomx_program_read_dataset_sshash_init
.set _rxrust_randomx_program_read_dataset_sshash_init, rxrust_randomx_program_read_dataset_sshash_init
.endif
.global rxrust_randomx_program_read_dataset_sshash_init_v2
.if {elf}
.hidden rxrust_randomx_program_read_dataset_sshash_init_v2
.endif
.if {macho}
.global _rxrust_randomx_program_read_dataset_sshash_init_v2
.alt_entry _rxrust_randomx_program_read_dataset_sshash_init_v2
.alt_entry rxrust_randomx_program_read_dataset_sshash_init_v2
.set _rxrust_randomx_program_read_dataset_sshash_init_v2, rxrust_randomx_program_read_dataset_sshash_init_v2
.endif
.global rxrust_randomx_program_read_dataset_sshash_fin
.if {elf}
.hidden rxrust_randomx_program_read_dataset_sshash_fin
.endif
.if {macho}
.global _rxrust_randomx_program_read_dataset_sshash_fin
.alt_entry _rxrust_randomx_program_read_dataset_sshash_fin
.alt_entry rxrust_randomx_program_read_dataset_sshash_fin
.set _rxrust_randomx_program_read_dataset_sshash_fin, rxrust_randomx_program_read_dataset_sshash_fin
.endif
.global rxrust_randomx_program_loop_store
.if {elf}
.hidden rxrust_randomx_program_loop_store
.endif
.if {macho}
.global _rxrust_randomx_program_loop_store
.alt_entry _rxrust_randomx_program_loop_store
.alt_entry rxrust_randomx_program_loop_store
.set _rxrust_randomx_program_loop_store, rxrust_randomx_program_loop_store
.endif
.global rxrust_randomx_program_loop_store_hard_aes
.if {elf}
.hidden rxrust_randomx_program_loop_store_hard_aes
.endif
.if {macho}
.global _rxrust_randomx_program_loop_store_hard_aes
.alt_entry _rxrust_randomx_program_loop_store_hard_aes
.alt_entry rxrust_randomx_program_loop_store_hard_aes
.set _rxrust_randomx_program_loop_store_hard_aes, rxrust_randomx_program_loop_store_hard_aes
.endif
.global rxrust_randomx_program_loop_store_soft_aes
.if {elf}
.hidden rxrust_randomx_program_loop_store_soft_aes
.endif
.if {macho}
.global _rxrust_randomx_program_loop_store_soft_aes
.alt_entry _rxrust_randomx_program_loop_store_soft_aes
.alt_entry rxrust_randomx_program_loop_store_soft_aes
.set _rxrust_randomx_program_loop_store_soft_aes, rxrust_randomx_program_loop_store_soft_aes
.endif
.global rxrust_randomx_program_loop_end
.if {elf}
.hidden rxrust_randomx_program_loop_end
.endif
.if {macho}
.global _rxrust_randomx_program_loop_end
.alt_entry _rxrust_randomx_program_loop_end
.alt_entry rxrust_randomx_program_loop_end
.set _rxrust_randomx_program_loop_end, rxrust_randomx_program_loop_end
.endif
.global rxrust_randomx_program_soft_aes
.if {elf}
.hidden rxrust_randomx_program_soft_aes
.endif
.if {macho}
.global _rxrust_randomx_program_soft_aes
.alt_entry _rxrust_randomx_program_soft_aes
.alt_entry rxrust_randomx_program_soft_aes
.set _rxrust_randomx_program_soft_aes, rxrust_randomx_program_soft_aes
.endif
.global rxrust_randomx_program_soft_aes_end
.if {elf}
.hidden rxrust_randomx_program_soft_aes_end
.endif
.if {macho}
.global _rxrust_randomx_program_soft_aes_end
.alt_entry _rxrust_randomx_program_soft_aes_end
.alt_entry rxrust_randomx_program_soft_aes_end
.set _rxrust_randomx_program_soft_aes_end, rxrust_randomx_program_soft_aes_end
.endif
.global rxrust_randomx_dataset_init
.if {elf}
.hidden rxrust_randomx_dataset_init
.endif
.if {macho}
.global _rxrust_randomx_dataset_init
.alt_entry _rxrust_randomx_dataset_init
.alt_entry rxrust_randomx_dataset_init
.set _rxrust_randomx_dataset_init, rxrust_randomx_dataset_init
.endif
.global rxrust_dataset_init_call_end
.if {elf}
.hidden rxrust_dataset_init_call_end
.endif
.if {macho}
.global _rxrust_dataset_init_call_end
.alt_entry _rxrust_dataset_init_call_end
.alt_entry rxrust_dataset_init_call_end
.set _rxrust_dataset_init_call_end, rxrust_dataset_init_call_end
.endif
.global rxrust_randomx_program_epilogue
.if {elf}
.hidden rxrust_randomx_program_epilogue
.endif
.if {macho}
.global _rxrust_randomx_program_epilogue
.alt_entry _rxrust_randomx_program_epilogue
.alt_entry rxrust_randomx_program_epilogue
.set _rxrust_randomx_program_epilogue, rxrust_randomx_program_epilogue
.endif
.global rxrust_randomx_sshash_load
.if {elf}
.hidden rxrust_randomx_sshash_load
.endif
.if {macho}
.global _rxrust_randomx_sshash_load
.alt_entry _rxrust_randomx_sshash_load
.alt_entry rxrust_randomx_sshash_load
.set _rxrust_randomx_sshash_load, rxrust_randomx_sshash_load
.endif
.global rxrust_randomx_sshash_prefetch
.if {elf}
.hidden rxrust_randomx_sshash_prefetch
.endif
.if {macho}
.global _rxrust_randomx_sshash_prefetch
.alt_entry _rxrust_randomx_sshash_prefetch
.alt_entry rxrust_randomx_sshash_prefetch
.set _rxrust_randomx_sshash_prefetch, rxrust_randomx_sshash_prefetch
.endif
.global rxrust_randomx_sshash_end
.if {elf}
.hidden rxrust_randomx_sshash_end
.endif
.if {macho}
.global _rxrust_randomx_sshash_end
.alt_entry _rxrust_randomx_sshash_end
.alt_entry rxrust_randomx_sshash_end
.set _rxrust_randomx_sshash_end, rxrust_randomx_sshash_end
.endif
.global rxrust_randomx_sshash_init
.if {elf}
.hidden rxrust_randomx_sshash_init
.endif
.if {macho}
.global _rxrust_randomx_sshash_init
.alt_entry _rxrust_randomx_sshash_init
.alt_entry rxrust_randomx_sshash_init
.set _rxrust_randomx_sshash_init, rxrust_randomx_sshash_init
.endif
.global rxrust_randomx_program_end
.if {elf}
.hidden rxrust_randomx_program_end
.endif
.if {macho}
.global _rxrust_randomx_program_end
.alt_entry _rxrust_randomx_program_end
.alt_entry rxrust_randomx_program_end
.set _rxrust_randomx_program_end, rxrust_randomx_program_end
.endif
.global rxrust_sshash_prefetch_hint
.if {elf}
.hidden rxrust_sshash_prefetch_hint
.endif
.if {macho}
.global _rxrust_sshash_prefetch_hint
.alt_entry _rxrust_sshash_prefetch_hint
.alt_entry rxrust_sshash_prefetch_hint
.set _rxrust_sshash_prefetch_hint, rxrust_sshash_prefetch_hint
.endif
.global rxrust_sshash_init_hint
.if {elf}
.hidden rxrust_sshash_init_hint
.endif
.if {macho}
.global _rxrust_sshash_init_hint
.alt_entry _rxrust_sshash_init_hint
.alt_entry rxrust_sshash_init_hint
.set _rxrust_sshash_init_hint, rxrust_sshash_init_hint
.endif
# This fragment is copied to a page-aligned JIT address. Its internal aligned
# constants must therefore have aligned offsets from this label, independently
# of preceding objects/assembler fragments in the source .rodata section.
.balign 64
rxrust_randomx_program_prologue:

	sub rsp, 456
	mov qword ptr [rsp+224], rbx
	mov qword ptr [rsp+232], rbp
	mov qword ptr [rsp+256], r12
	mov qword ptr [rsp+264], r13
	mov qword ptr [rsp+272], r14
	mov qword ptr [rsp+280], r15


	mov rbx, rcx
	mov qword ptr [rsp+448], rdi
	mov rcx, rdi
	mov rbp, qword ptr [rsi]
	mov rdi, qword ptr [rsi+8]
	mov rsi, rdx

	mov rax, rbp
	mov r8d, ebp
	ror rbp, 32
	mov r9d, ebp

	and r8d, 2147483584
	and r9d, 2147483584
	prefetchnta byte ptr [rdi+r8]
	prefetchnta byte ptr [rdi+r9]


	xor r8, r8
	xor r9, r9
	xor r10, r10
	xor r11, r11
	xor r12, r12
	xor r13, r13
	xor r14, r14
	xor r15, r15


	lea rcx, [rcx+120]
	movapd xmm8, xmmword ptr [rcx+72]
	movapd xmm9, xmmword ptr [rcx+88]
	movapd xmm10, xmmword ptr [rcx+104]
	movapd xmm11, xmmword ptr [rcx+120]



	mov dword ptr [rsp+4], -1


	mov rdx, 0x0000bfc000009fc0
	mov qword ptr [rsp+32], rdx
	mov rdx, 0x0000ffc00000dfc0
	mov qword ptr [rsp+40], rdx
	movapd xmm13, xmmword ptr [mantissaMask+rip]
	movapd xmm14, xmmword ptr [rxrust_exp240+rip]
	movapd xmm15, xmmword ptr [rxrust_scaleMask+rip]
	mov rdx, rax
	and eax, 2097088
	ror rdx, 32
	and edx, 2097088
	jmp rxrust_rx_program_loop_begin

.balign 64
	mantissaMask:
	.byte 255, 255, 255, 255, 255, 255, 255, 0, 255, 255, 255, 255, 255, 255, 255, 0
rxrust_exp240:
	.byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
rxrust_scaleMask:
	.byte 0, 0, 0, 0, 0, 0, 240, 128, 0, 0, 0, 0, 0, 0, 240, 128


.balign 64
rxrust_randomx_program_loop_begin:
rxrust_rx_program_loop_begin:
	nop

rxrust_randomx_program_loop_load:
		lea rcx, [rsi+rax]
	mov qword ptr [rsp+8], rcx
	xor r8,  qword ptr [rcx+0]
	xor r9,  qword ptr [rcx+8]
	xor r10, qword ptr [rcx+16]
	xor r11, qword ptr [rcx+24]
	xor r12, qword ptr [rcx+32]
	xor r13, qword ptr [rcx+40]
	xor r14, qword ptr [rcx+48]
	xor r15, qword ptr [rcx+56]
	lea rcx, [rsi+rdx]
	mov qword ptr [rsp+16], rcx
	cvtdq2pd xmm0, qword ptr [rcx+0]
	cvtdq2pd xmm1, qword ptr [rcx+8]
	cvtdq2pd xmm2, qword ptr [rcx+16]
	cvtdq2pd xmm3, qword ptr [rcx+24]
	cvtdq2pd xmm4, qword ptr [rcx+32]
	cvtdq2pd xmm5, qword ptr [rcx+40]
	cvtdq2pd xmm6, qword ptr [rcx+48]
	cvtdq2pd xmm7, qword ptr [rcx+56]
	andps xmm4, xmm13
	andps xmm5, xmm13
	andps xmm6, xmm13
	andps xmm7, xmm13
	orps xmm4, xmm14
	orps xmm5, xmm14
	orps xmm6, xmm14
	orps xmm7, xmm14



rxrust_randomx_program_start:
	nop

rxrust_randomx_program_read_dataset:
		mov ecx, ebp
	and ecx, 2147483584
	xor r8, qword ptr [rdi+rcx]
	ror rbp, 32
	xor rbp, rax
	mov edx, ebp
	and edx, 2147483584
	prefetchnta byte ptr [rdi+rdx]
	xor r9,  qword ptr [rdi+rcx+8]
	xor r10, qword ptr [rdi+rcx+16]
	xor r11, qword ptr [rdi+rcx+24]
	xor r12, qword ptr [rdi+rcx+32]
	xor r13, qword ptr [rdi+rcx+40]
	xor r14, qword ptr [rdi+rcx+48]
	xor r15, qword ptr [rdi+rcx+56]



rxrust_randomx_program_read_dataset_v2:
		mov ecx, ebp
	and ecx, 2147483584
	xor r8, qword ptr [rdi+rcx]
	xor rbp, rax
	mov edx, ebp
	ror rbp, 32
	and edx, 2147483584
	prefetchnta byte ptr [rdi+rdx]
	xor r9,  qword ptr [rdi+rcx+8]
	xor r10, qword ptr [rdi+rcx+16]
	xor r11, qword ptr [rdi+rcx+24]
	xor r12, qword ptr [rdi+rcx+32]
	xor r13, qword ptr [rdi+rcx+40]
	xor r14, qword ptr [rdi+rcx+48]
	xor r15, qword ptr [rdi+rcx+56]



rxrust_randomx_program_read_dataset_sshash_init:
		mov qword ptr [rsp+24], r8
	mov qword ptr [rsp+32], r9
	mov qword ptr [rsp+40], r10
	mov qword ptr [rsp+48], r11
	mov qword ptr [rsp+56], r12
	mov qword ptr [rsp+64], r13
	mov qword ptr [rsp+72], r14
	mov qword ptr [rsp+80], r15
	mov qword ptr [rsp+88], rbx
	mov ebx, ebp
	shr ebx, 6
	and ebx, 2147483584 / 64
	ror rbp, 32
	xor rbp, rax




rxrust_randomx_program_read_dataset_sshash_init_v2:
		mov qword ptr [rsp+24], r8
	mov qword ptr [rsp+32], r9
	mov qword ptr [rsp+40], r10
	mov qword ptr [rsp+48], r11
	mov qword ptr [rsp+56], r12
	mov qword ptr [rsp+64], r13
	mov qword ptr [rsp+72], r14
	mov qword ptr [rsp+80], r15
	mov qword ptr [rsp+88], rbx
	mov ebx, ebp
	shr ebx, 6
	and ebx, 2147483584 / 64
	xor rbp, rax
	ror rbp, 32




rxrust_randomx_program_read_dataset_sshash_fin:
		xor r8, qword ptr [rsp+24]
	xor r9, qword ptr [rsp+32]
	xor r10, qword ptr [rsp+40]
	xor r11, qword ptr [rsp+48]
	xor r12, qword ptr [rsp+56]
	xor r13, qword ptr [rsp+64]
	xor r14, qword ptr [rsp+72]
	xor r15, qword ptr [rsp+80]
	mov rbx, qword ptr [rsp+88]



rxrust_randomx_program_loop_store:
		mov rcx, qword ptr [rsp+16]
	mov qword ptr [rcx+0], r8
	mov qword ptr [rcx+8], r9
	mov qword ptr [rcx+16], r10
	mov qword ptr [rcx+24], r11
	mov qword ptr [rcx+32], r12
	mov qword ptr [rcx+40], r13
	mov qword ptr [rcx+48], r14
	mov qword ptr [rcx+56], r15
	mov rcx, qword ptr [rsp+8]
	xorpd xmm0, xmm4
	xorpd xmm1, xmm5
	xorpd xmm2, xmm6
	xorpd xmm3, xmm7
	movapd xmmword ptr [rcx+0], xmm0
	movapd xmmword ptr [rcx+16], xmm1
	movapd xmmword ptr [rcx+32], xmm2
	movapd xmmword ptr [rcx+48], xmm3



rxrust_randomx_program_loop_store_hard_aes:
		mov rcx, qword ptr [rsp+16]
	mov qword ptr [rcx+0], r8
	mov qword ptr [rcx+8], r9
	mov qword ptr [rcx+16], r10
	mov qword ptr [rcx+24], r11
	mov qword ptr [rcx+32], r12
	mov qword ptr [rcx+40], r13
	mov qword ptr [rcx+48], r14
	mov qword ptr [rcx+56], r15
	mov rcx, qword ptr [rsp+8]
	aesenc xmm0, xmm4
	aesdec xmm1, xmm4
	aesenc xmm2, xmm4
	aesdec xmm3, xmm4
	aesenc xmm0, xmm5
	aesdec xmm1, xmm5
	aesenc xmm2, xmm5
	aesdec xmm3, xmm5
	aesenc xmm0, xmm6
	aesdec xmm1, xmm6
	aesenc xmm2, xmm6
	aesdec xmm3, xmm6
	aesenc xmm0, xmm7
	aesdec xmm1, xmm7
	aesenc xmm2, xmm7
	aesdec xmm3, xmm7
	movapd xmmword ptr [rcx+0], xmm0
	movapd xmmword ptr [rcx+16], xmm1
	movapd xmmword ptr [rcx+32], xmm2
	movapd xmmword ptr [rcx+48], xmm3



rxrust_randomx_program_loop_store_soft_aes:
		mov rcx, qword ptr [rsp+16]
	mov qword ptr [rcx+0], r8
	mov qword ptr [rcx+8], r9
	mov qword ptr [rcx+16], r10
	mov qword ptr [rcx+24], r11
	mov qword ptr [rcx+32], r12
	mov qword ptr [rcx+40], r13
	mov qword ptr [rcx+48], r14
	mov qword ptr [rcx+56], r15
	mov rcx, qword ptr [rsp+8]

	mov qword ptr [rsp+24], rax
	mov qword ptr [rsp+40], rcx
	mov qword ptr [rsp+48], rdx
	mov qword ptr [rsp+56], rbp
	mov qword ptr [rsp+72], rsi
	mov qword ptr [rsp+80], rdi
	mov qword ptr [rsp+88], rbx
	movdqa xmmword ptr [rsp+160], xmm13
	movdqa xmmword ptr [rsp+176], xmm14
	mov rdi, qword ptr [rxrust_aes_lut_enc+rip]
	mov rsi, qword ptr [rxrust_aes_lut_dec+rip]
	movapd xmm12, xmm4
	call rxrust_soft_aes_all
	movapd xmm12, xmm5
	call rxrust_soft_aes_all
	movapd xmm12, xmm6
	call rxrust_soft_aes_all
	movapd xmm12, xmm7
	call rxrust_soft_aes_all
# Restore R8 from the integer store before any floating-point store; the two
# scratchpad addresses may coincide. Other integer registers stay untouched.
	mov rcx, qword ptr [rsp+16]
	mov r8, qword ptr [rcx]
	mov rax, qword ptr [rsp+24]
	mov rcx, qword ptr [rsp+40]
	mov rdx, qword ptr [rsp+48]
	mov rbp, qword ptr [rsp+56]
	mov rsi, qword ptr [rsp+72]
	mov rdi, qword ptr [rsp+80]
	mov rbx, qword ptr [rsp+88]
	movdqa xmm13, xmmword ptr [rsp+160]
	movdqa xmm14, xmmword ptr [rsp+176]
	movapd xmmword ptr [rcx+0], xmm0
	movapd xmmword ptr [rcx+16], xmm1
	movapd xmmword ptr [rcx+32], xmm2
	movapd xmmword ptr [rcx+48], xmm3

rxrust_randomx_program_loop_end:
	sub ebx, 1
	jnz rxrust_rx_program_end
	jmp rxrust_rx_program_end

# SSE2 lookup rounds keep the four independent F lanes in XMM0..3.
# XMM4..7 hold E keys; XMM12 carries one selected key across all four lanes.
# XMM13/14 and R8 are temporarily borrowed and restored by the caller.
# Each helper call applies one E key to all four F lanes.
# Only table loads touch memory inside a round. No AES-NI or SSSE3 is used.
.macro rxrust_soft_dec_round state, table
	movq rdx, \state
	movzx ecx, dl
	mov eax, dword ptr [\table+rcx*4+0]
	movzx ecx, dh
	mov ebx, dword ptr [\table+rcx*4+1024]
	shr rdx, 16
	movzx ecx, dl
	mov ebp, dword ptr [\table+rcx*4+2048]
	movzx ecx, dh
	mov r8d, dword ptr [\table+rcx*4+3072]
	shr rdx, 16
	movzx ecx, dl
	xor ebx, dword ptr [\table+rcx*4+0]
	movzx ecx, dh
	xor ebp, dword ptr [\table+rcx*4+1024]
	shr rdx, 16
	movzx ecx, dl
	xor r8d, dword ptr [\table+rcx*4+2048]
	movzx ecx, dh
	xor eax, dword ptr [\table+rcx*4+3072]
	psrldq \state, 8
	movq rdx, \state
	movzx ecx, dl
	xor ebp, dword ptr [\table+rcx*4+0]
	movzx ecx, dh
	xor r8d, dword ptr [\table+rcx*4+1024]
	shr rdx, 16
	movzx ecx, dl
	xor eax, dword ptr [\table+rcx*4+2048]
	movzx ecx, dh
	xor ebx, dword ptr [\table+rcx*4+3072]
	shr rdx, 16
	movzx ecx, dl
	xor r8d, dword ptr [\table+rcx*4+0]
	movzx ecx, dh
	xor eax, dword ptr [\table+rcx*4+1024]
	shr rdx, 16
	movzx ecx, dl
	xor ebx, dword ptr [\table+rcx*4+2048]
	movzx ecx, dh
	xor ebp, dword ptr [\table+rcx*4+3072]
	movd \state, eax
	movd xmm13, ebx
	punpckldq \state, xmm13
	movd xmm13, ebp
	movd xmm14, r8d
	punpckldq xmm13, xmm14
	punpcklqdq \state, xmm13
.endm

.macro rxrust_soft_enc_round state, table
	movq rdx, \state
	movzx ecx, dl
	mov eax, dword ptr [\table+rcx*4+0]
	movzx ecx, dh
	mov r8d, dword ptr [\table+rcx*4+1024]
	shr rdx, 16
	movzx ecx, dl
	mov ebp, dword ptr [\table+rcx*4+2048]
	movzx ecx, dh
	mov ebx, dword ptr [\table+rcx*4+3072]
	shr rdx, 16
	movzx ecx, dl
	xor ebx, dword ptr [\table+rcx*4+0]
	movzx ecx, dh
	xor eax, dword ptr [\table+rcx*4+1024]
	shr rdx, 16
	movzx ecx, dl
	xor r8d, dword ptr [\table+rcx*4+2048]
	movzx ecx, dh
	xor ebp, dword ptr [\table+rcx*4+3072]
	psrldq \state, 8
	movq rdx, \state
	movzx ecx, dl
	xor ebp, dword ptr [\table+rcx*4+0]
	movzx ecx, dh
	xor ebx, dword ptr [\table+rcx*4+1024]
	shr rdx, 16
	movzx ecx, dl
	xor eax, dword ptr [\table+rcx*4+2048]
	movzx ecx, dh
	xor r8d, dword ptr [\table+rcx*4+3072]
	shr rdx, 16
	movzx ecx, dl
	xor r8d, dword ptr [\table+rcx*4+0]
	movzx ecx, dh
	xor ebp, dword ptr [\table+rcx*4+1024]
	shr rdx, 16
	movzx ecx, dl
	xor ebx, dword ptr [\table+rcx*4+2048]
	movzx ecx, dh
	xor eax, dword ptr [\table+rcx*4+3072]
	movd \state, eax
	movd xmm13, ebx
	punpckldq \state, xmm13
	movd xmm13, ebp
	movd xmm14, r8d
	punpckldq xmm13, xmm14
	punpcklqdq \state, xmm13
.endm

rxrust_randomx_program_soft_aes:
rxrust_soft_aes_all:
	rxrust_soft_enc_round xmm0, rdi
	pxor xmm0, xmm12
	rxrust_soft_dec_round xmm1, rsi
	pxor xmm1, xmm12
	rxrust_soft_enc_round xmm2, rdi
	pxor xmm2, xmm12
	rxrust_soft_dec_round xmm3, rsi
	pxor xmm3, xmm12
	ret

.purgem rxrust_soft_dec_round
.purgem rxrust_soft_enc_round

rxrust_randomx_program_soft_aes_end:
rxrust_aes_lut_enc:
	.byte 0, 0, 0, 0, 0, 0, 0, 0
rxrust_aes_lut_dec:
	.byte 0, 0, 0, 0, 0, 0, 0, 0

.balign 64
rxrust_randomx_dataset_init:
rxrust_rx_dataset_init:
	push rbx
	push rbp
	push r12
	push r13
	push r14
	push r15
	mov rdi, qword ptr [rdi]

	mov rbp, rdx
	push rcx
rxrust_init_block_loop:
	prefetchw byte ptr [rsi]
	mov rbx, rbp
	.byte 232
	.int 16384 - (rxrust_dataset_init_call_end - rxrust_rx_dataset_init)
rxrust_dataset_init_call_end:
	mov qword ptr [rsi+0], r8
	mov qword ptr [rsi+8], r9
	mov qword ptr [rsi+16], r10
	mov qword ptr [rsi+24], r11
	mov qword ptr [rsi+32], r12
	mov qword ptr [rsi+40], r13
	mov qword ptr [rsi+48], r14
	mov qword ptr [rsi+56], r15
	add rbp, 1
	add rsi, 64
	cmp rbp, qword ptr [rsp]
	jb rxrust_init_block_loop
	pop rax
	pop r15
	pop r14
	pop r13
	pop r12
	pop rbp
	pop rbx
	ret

.balign 64
rxrust_randomx_program_epilogue:

	mov rcx, qword ptr [rsp+448]
	mov qword ptr [rcx+0], r8
	mov qword ptr [rcx+8], r9
	mov qword ptr [rcx+16], r10
	mov qword ptr [rcx+24], r11
	mov qword ptr [rcx+32], r12
	mov qword ptr [rcx+40], r13
	mov qword ptr [rcx+48], r14
	mov qword ptr [rcx+56], r15
	movdqa xmmword ptr [rcx+64], xmm0
	movdqa xmmword ptr [rcx+80], xmm1
	movdqa xmmword ptr [rcx+96], xmm2
	movdqa xmmword ptr [rcx+112], xmm3
	lea rcx, [rcx+64]
	movdqa xmmword ptr [rcx+64], xmm4
	movdqa xmmword ptr [rcx+80], xmm5
	movdqa xmmword ptr [rcx+96], xmm6
	movdqa xmmword ptr [rcx+112], xmm7


	mov r15, qword ptr [rsp+280]
	mov r14, qword ptr [rsp+272]
	mov r13, qword ptr [rsp+264]
	mov r12, qword ptr [rsp+256]
	mov rbp, qword ptr [rsp+232]
	mov rbx, qword ptr [rsp+224]

	add rsp, 456


	ret



.balign 64
rxrust_randomx_sshash_load:
		xor r8, qword ptr [rbx+0]
	xor r9, qword ptr [rbx+8]
	xor r10, qword ptr [rbx+16]
	xor r11, qword ptr [rbx+24]
	xor r12, qword ptr [rbx+32]
	xor r13, qword ptr [rbx+40]
	xor r14, qword ptr [rbx+48]
	xor r15, qword ptr [rbx+56]


rxrust_randomx_sshash_prefetch:
		and rbx, 4194303
	shl rbx, 6
	add rbx, rdi
rxrust_sshash_prefetch_hint:
	prefetchnta byte ptr [rbx]


rxrust_randomx_sshash_end:
	nop

.balign 64
rxrust_randomx_sshash_init:
	lea r8, [rbx+1]
		and rbx, 4194303
	shl rbx, 6
	add rbx, rdi
rxrust_sshash_init_hint:
	prefetchnta byte ptr [rbx]

	imul r8, qword ptr [r0_mul+rip]
	mov r9, qword ptr [rxrust_r1_add+rip]
	xor r9, r8
	mov r10, qword ptr [rxrust_r2_add+rip]
	xor r10, r8
	mov r11, qword ptr [rxrust_r3_add+rip]
	xor r11, r8
	mov r12, qword ptr [rxrust_r4_add+rip]
	xor r12, r8
	mov r13, qword ptr [rxrust_r5_add+rip]
	xor r13, r8
	mov r14, qword ptr [rxrust_r6_add+rip]
	xor r14, r8
	mov r15, qword ptr [rxrust_r7_add+rip]
	xor r15, r8
	jmp rxrust_rx_program_end

.balign 64
	r0_mul:

	.byte 45, 127, 149, 76, 45, 244, 81, 88
rxrust_r1_add:

	.byte 252, 161, 245, 89, 138, 151, 10, 129
rxrust_r2_add:

	.byte 70, 216, 194, 56, 223, 153, 112, 167
rxrust_r3_add:

	.byte 92, 73, 34, 191, 28, 185, 38, 129
rxrust_r4_add:

	.byte 98, 138, 159, 23, 151, 37, 77, 73
rxrust_r5_add:

	.byte 12, 236, 170, 206, 185, 239, 55, 146
rxrust_r6_add:

	.byte 120, 45, 230, 108, 116, 86, 42, 47
rxrust_r7_add:

	.byte 78, 229, 44, 182, 247, 59, 133, 132


.balign 64
rxrust_randomx_program_end:
rxrust_rx_program_end:
	nop

.text
"#,
    macho = const cfg!(target_vendor = "apple") as u8,
    elf = const cfg!(all(unix, not(target_vendor = "apple"))) as u8,
);

unsafe extern "C" {
    static rxrust_randomx_program_prologue: u8;
    static rxrust_randomx_program_loop_begin: u8;
    static rxrust_randomx_program_loop_load: u8;
    static rxrust_randomx_program_start: u8;
    static rxrust_randomx_program_read_dataset: u8;
    static rxrust_randomx_program_read_dataset_v2: u8;
    static rxrust_randomx_program_read_dataset_sshash_init: u8;
    static rxrust_randomx_program_read_dataset_sshash_init_v2: u8;
    static rxrust_randomx_program_read_dataset_sshash_fin: u8;
    static rxrust_randomx_program_loop_store: u8;
    static rxrust_randomx_program_loop_store_hard_aes: u8;
    static rxrust_randomx_program_loop_store_soft_aes: u8;
    static rxrust_randomx_program_loop_end: u8;
    static rxrust_randomx_program_soft_aes: u8;
    static rxrust_randomx_program_soft_aes_end: u8;
    static rxrust_randomx_dataset_init: u8;
    static rxrust_dataset_init_call_end: u8;
    static rxrust_randomx_program_epilogue: u8;
    static rxrust_randomx_sshash_load: u8;
    static rxrust_randomx_sshash_prefetch: u8;
    static rxrust_randomx_sshash_end: u8;
    static rxrust_randomx_sshash_init: u8;
    static rxrust_sshash_prefetch_hint: u8;
    static rxrust_sshash_init_hint: u8;
    static rxrust_randomx_program_end: u8;
}

fn part(start: *const u8, end: *const u8) -> &'static [u8] {
    let len = end
        .addr()
        .checked_sub(start.addr())
        .expect("template order");
    // SAFETY: both symbols delimit one immutable assembler template in this
    // module. The linker retains their order within a single rodata section.
    unsafe { std::slice::from_raw_parts(start, len) }
}
pub(super) fn prologue() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_prologue,
        &raw const rxrust_randomx_program_loop_begin,
    )
}
pub(super) fn load() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_loop_load,
        &raw const rxrust_randomx_program_start,
    )
}
pub(super) fn dataset_v1() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_read_dataset,
        &raw const rxrust_randomx_program_read_dataset_v2,
    )
}
pub(super) fn dataset_v2() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_read_dataset_v2,
        &raw const rxrust_randomx_program_read_dataset_sshash_init,
    )
}
pub(super) fn light_v1() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_read_dataset_sshash_init,
        &raw const rxrust_randomx_program_read_dataset_sshash_init_v2,
    )
}
pub(super) fn light_v2() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_read_dataset_sshash_init_v2,
        &raw const rxrust_randomx_program_read_dataset_sshash_fin,
    )
}
pub(super) fn light_finish() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_read_dataset_sshash_fin,
        &raw const rxrust_randomx_program_loop_store,
    )
}
pub(super) fn store_v1() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_loop_store,
        &raw const rxrust_randomx_program_loop_store_hard_aes,
    )
}
pub(super) fn store_v2() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_loop_store_hard_aes,
        &raw const rxrust_randomx_program_loop_store_soft_aes,
    )
}
pub(super) fn store_soft_v2() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_loop_store_soft_aes,
        &raw const rxrust_randomx_program_loop_end,
    )
}
pub(super) fn soft_aes() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_soft_aes,
        &raw const rxrust_randomx_program_soft_aes_end,
    )
}
pub(super) fn dataset_init() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_dataset_init,
        &raw const rxrust_randomx_program_epilogue,
    )
}
/// Offset immediately after the initializer's sole relative helper call.
pub(super) fn dataset_init_call_end() -> usize {
    part(
        &raw const rxrust_randomx_dataset_init,
        &raw const rxrust_dataset_init_call_end,
    )
    .len()
}
pub(super) fn epilogue() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_program_epilogue,
        &raw const rxrust_randomx_sshash_load,
    )
}
pub(super) fn ss_load() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_sshash_load,
        &raw const rxrust_randomx_sshash_prefetch,
    )
}
pub(super) fn ss_prefetch() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_sshash_prefetch,
        &raw const rxrust_randomx_sshash_end,
    )
}
pub(super) fn ss_init() -> &'static [u8] {
    part(
        &raw const rxrust_randomx_sshash_init,
        &raw const rxrust_randomx_program_end,
    )
}

pub(super) fn ss_init_hint_offset() -> usize {
    part(
        &raw const rxrust_randomx_sshash_init,
        &raw const rxrust_sshash_init_hint,
    )
    .len()
}
pub(super) fn ss_prefetch_hint_offset() -> usize {
    part(
        &raw const rxrust_randomx_sshash_prefetch,
        &raw const rxrust_sshash_prefetch_hint,
    )
    .len()
}

/// Invariant RIP-relative loads followed by stores into the VM stack slots.
pub(super) fn stack_reciprocals() -> &'static [u8; 192] {
    static PREFIX: std::sync::OnceLock<[u8; 192]> = std::sync::OnceLock::new();
    PREFIX.get_or_init(|| {
        let mut code = [0; 192];
        let start = prologue().len();
        for slot in 0..16 {
            let at = slot * 12;
            code[at..at + 3].copy_from_slice(&[0x48, 0x8b, 0x0d]);
            let relative = (super::x86_64::RECIPROCALS + 8 * slot) as i32 - (start + at + 7) as i32;
            code[at + 3..at + 7].copy_from_slice(&relative.to_le_bytes());
            code[at + 7..at + 11].copy_from_slice(&[0x48, 0x89, 0x4c, 0x24]);
            code[at + 11] = (-8 * (slot as i32 + 1)) as u8;
        }
        code
    })
}
