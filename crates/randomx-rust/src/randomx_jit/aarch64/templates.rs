// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2019, SChernykh <https://github.com/SChernykh>
// Copyright (c) 2026 RxMiner contributors
// Rust integrated ARM64 templates, ported from RandomX. See ../../randomx_core/LICENSE.
core::arch::global_asm!(r#"
	.arch armv8-a+crypto
.if {macho}
// Keep template aliases separate from LLVM's constant pools and jump tables.
// Sharing __const can make their relocations target the template section label,
// which Apple's linker rejects when linking the crate's test executable.
.section __TEXT,__rxarm,regular
.else
.section .rodata
.endif
	.global rxarm_randomx_program_aarch64
.set rxarm_randomx_program_aarch64, Lrxarm_randomx_program_aarch64
.if {macho}
.global _rxarm_randomx_program_aarch64
.set _rxarm_randomx_program_aarch64, rxarm_randomx_program_aarch64
.endif
	.global rxarm_randomx_program_aarch64_main_loop
.set rxarm_randomx_program_aarch64_main_loop, Lrxarm_randomx_program_aarch64_main_loop
.if {macho}
.global _rxarm_randomx_program_aarch64_main_loop
.alt_entry _rxarm_randomx_program_aarch64_main_loop
.alt_entry rxarm_randomx_program_aarch64_main_loop
.set _rxarm_randomx_program_aarch64_main_loop, rxarm_randomx_program_aarch64_main_loop
.endif
	.global rxarm_randomx_program_aarch64_vm_instructions
.set rxarm_randomx_program_aarch64_vm_instructions, Lrxarm_randomx_program_aarch64_vm_instructions
.if {macho}
.global _rxarm_randomx_program_aarch64_vm_instructions
.alt_entry _rxarm_randomx_program_aarch64_vm_instructions
.alt_entry rxarm_randomx_program_aarch64_vm_instructions
.set _rxarm_randomx_program_aarch64_vm_instructions, rxarm_randomx_program_aarch64_vm_instructions
.endif
	.global rxarm_randomx_program_aarch64_imul_rcp_literals_end
.set rxarm_randomx_program_aarch64_imul_rcp_literals_end, Lrxarm_randomx_program_aarch64_imul_rcp_literals_end
.if {macho}
.global _rxarm_randomx_program_aarch64_imul_rcp_literals_end
.alt_entry _rxarm_randomx_program_aarch64_imul_rcp_literals_end
.alt_entry rxarm_randomx_program_aarch64_imul_rcp_literals_end
.set _rxarm_randomx_program_aarch64_imul_rcp_literals_end, rxarm_randomx_program_aarch64_imul_rcp_literals_end
.endif
	.global rxarm_randomx_program_aarch64_vm_instructions_end
.set rxarm_randomx_program_aarch64_vm_instructions_end, Lrxarm_randomx_program_aarch64_vm_instructions_end
.if {macho}
.global _rxarm_randomx_program_aarch64_vm_instructions_end
.alt_entry _rxarm_randomx_program_aarch64_vm_instructions_end
.alt_entry rxarm_randomx_program_aarch64_vm_instructions_end
.set _rxarm_randomx_program_aarch64_vm_instructions_end, rxarm_randomx_program_aarch64_vm_instructions_end
.endif
	.global rxarm_randomx_program_aarch64_cacheline_align_mask1
.set rxarm_randomx_program_aarch64_cacheline_align_mask1, Lrxarm_randomx_program_aarch64_cacheline_align_mask1
.if {macho}
.global _rxarm_randomx_program_aarch64_cacheline_align_mask1
.alt_entry _rxarm_randomx_program_aarch64_cacheline_align_mask1
.alt_entry rxarm_randomx_program_aarch64_cacheline_align_mask1
.set _rxarm_randomx_program_aarch64_cacheline_align_mask1, rxarm_randomx_program_aarch64_cacheline_align_mask1
.endif
	.global rxarm_randomx_program_aarch64_cacheline_align_mask2
.set rxarm_randomx_program_aarch64_cacheline_align_mask2, Lrxarm_randomx_program_aarch64_cacheline_align_mask2
.if {macho}
.global _rxarm_randomx_program_aarch64_cacheline_align_mask2
.alt_entry _rxarm_randomx_program_aarch64_cacheline_align_mask2
.alt_entry rxarm_randomx_program_aarch64_cacheline_align_mask2
.set _rxarm_randomx_program_aarch64_cacheline_align_mask2, rxarm_randomx_program_aarch64_cacheline_align_mask2
.endif
	.global rxarm_randomx_program_aarch64_update_spMix1
.set rxarm_randomx_program_aarch64_update_spMix1, Lrxarm_randomx_program_aarch64_update_spMix1
.if {macho}
.global _rxarm_randomx_program_aarch64_update_spMix1
.alt_entry _rxarm_randomx_program_aarch64_update_spMix1
.alt_entry rxarm_randomx_program_aarch64_update_spMix1
.set _rxarm_randomx_program_aarch64_update_spMix1, rxarm_randomx_program_aarch64_update_spMix1
.endif
	.global rxarm_randomx_program_aarch64_v2_FE_mix
.set rxarm_randomx_program_aarch64_v2_FE_mix, Lrxarm_randomx_program_aarch64_v2_FE_mix
.if {macho}
.global _rxarm_randomx_program_aarch64_v2_FE_mix
.alt_entry _rxarm_randomx_program_aarch64_v2_FE_mix
.alt_entry rxarm_randomx_program_aarch64_v2_FE_mix
.set _rxarm_randomx_program_aarch64_v2_FE_mix, rxarm_randomx_program_aarch64_v2_FE_mix
.endif
	.global rxarm_randomx_program_aarch64_v1_FE_mix
.set rxarm_randomx_program_aarch64_v1_FE_mix, Lrxarm_randomx_program_aarch64_v1_FE_mix
.if {macho}
.global _rxarm_randomx_program_aarch64_v1_FE_mix
.alt_entry _rxarm_randomx_program_aarch64_v1_FE_mix
.alt_entry rxarm_randomx_program_aarch64_v1_FE_mix
.set _rxarm_randomx_program_aarch64_v1_FE_mix, rxarm_randomx_program_aarch64_v1_FE_mix
.endif
	.global rxarm_randomx_program_aarch64_v2_FE_mix_soft_aes
.set rxarm_randomx_program_aarch64_v2_FE_mix_soft_aes, Lrxarm_randomx_program_aarch64_v2_FE_mix_soft_aes
.if {macho}
.global _rxarm_randomx_program_aarch64_v2_FE_mix_soft_aes
.alt_entry _rxarm_randomx_program_aarch64_v2_FE_mix_soft_aes
.alt_entry rxarm_randomx_program_aarch64_v2_FE_mix_soft_aes
.set _rxarm_randomx_program_aarch64_v2_FE_mix_soft_aes, rxarm_randomx_program_aarch64_v2_FE_mix_soft_aes
.endif
	.if !{macho}
	.hidden rxarm_randomx_program_aarch64_aes_lut_pointers
	.endif
	.global rxarm_randomx_program_aarch64_aes_lut_pointers
.set rxarm_randomx_program_aarch64_aes_lut_pointers, Lrxarm_randomx_program_aarch64_aes_lut_pointers
.if {macho}
.global _rxarm_randomx_program_aarch64_aes_lut_pointers
.alt_entry _rxarm_randomx_program_aarch64_aes_lut_pointers
.alt_entry rxarm_randomx_program_aarch64_aes_lut_pointers
.set _rxarm_randomx_program_aarch64_aes_lut_pointers, rxarm_randomx_program_aarch64_aes_lut_pointers
.endif
	.global rxarm_randomx_program_aarch64_vm_instructions_end_light
.set rxarm_randomx_program_aarch64_vm_instructions_end_light, Lrxarm_randomx_program_aarch64_vm_instructions_end_light
.if {macho}
.global _rxarm_randomx_program_aarch64_vm_instructions_end_light
.alt_entry _rxarm_randomx_program_aarch64_vm_instructions_end_light
.alt_entry rxarm_randomx_program_aarch64_vm_instructions_end_light
.set _rxarm_randomx_program_aarch64_vm_instructions_end_light, rxarm_randomx_program_aarch64_vm_instructions_end_light
.endif
	.global rxarm_randomx_program_aarch64_vm_instructions_end_light_tweak
.set rxarm_randomx_program_aarch64_vm_instructions_end_light_tweak, Lrxarm_randomx_program_aarch64_vm_instructions_end_light_tweak
.if {macho}
.global _rxarm_randomx_program_aarch64_vm_instructions_end_light_tweak
.alt_entry _rxarm_randomx_program_aarch64_vm_instructions_end_light_tweak
.alt_entry rxarm_randomx_program_aarch64_vm_instructions_end_light_tweak
.set _rxarm_randomx_program_aarch64_vm_instructions_end_light_tweak, rxarm_randomx_program_aarch64_vm_instructions_end_light_tweak
.endif
	.global rxarm_randomx_program_aarch64_light_cacheline_align_mask
.set rxarm_randomx_program_aarch64_light_cacheline_align_mask, Lrxarm_randomx_program_aarch64_light_cacheline_align_mask
.if {macho}
.global _rxarm_randomx_program_aarch64_light_cacheline_align_mask
.alt_entry _rxarm_randomx_program_aarch64_light_cacheline_align_mask
.alt_entry rxarm_randomx_program_aarch64_light_cacheline_align_mask
.set _rxarm_randomx_program_aarch64_light_cacheline_align_mask, rxarm_randomx_program_aarch64_light_cacheline_align_mask
.endif
	.global rxarm_randomx_program_aarch64_light_dataset_offset
.set rxarm_randomx_program_aarch64_light_dataset_offset, Lrxarm_randomx_program_aarch64_light_dataset_offset
.if {macho}
.global _rxarm_randomx_program_aarch64_light_dataset_offset
.alt_entry _rxarm_randomx_program_aarch64_light_dataset_offset
.alt_entry rxarm_randomx_program_aarch64_light_dataset_offset
.set _rxarm_randomx_program_aarch64_light_dataset_offset, rxarm_randomx_program_aarch64_light_dataset_offset
.endif
	.global rxarm_randomx_program_aarch64_vm_instructions_end_v1
.set rxarm_randomx_program_aarch64_vm_instructions_end_v1, Lrxarm_randomx_program_aarch64_vm_instructions_end_v1
.if {macho}
.global _rxarm_randomx_program_aarch64_vm_instructions_end_v1
.alt_entry _rxarm_randomx_program_aarch64_vm_instructions_end_v1
.alt_entry rxarm_randomx_program_aarch64_vm_instructions_end_v1
.set _rxarm_randomx_program_aarch64_vm_instructions_end_v1, rxarm_randomx_program_aarch64_vm_instructions_end_v1
.endif
	.global rxarm_randomx_program_aarch64_vm_instructions_end_v2
.set rxarm_randomx_program_aarch64_vm_instructions_end_v2, Lrxarm_randomx_program_aarch64_vm_instructions_end_v2
.if {macho}
.global _rxarm_randomx_program_aarch64_vm_instructions_end_v2
.alt_entry _rxarm_randomx_program_aarch64_vm_instructions_end_v2
.alt_entry rxarm_randomx_program_aarch64_vm_instructions_end_v2
.set _rxarm_randomx_program_aarch64_vm_instructions_end_v2, rxarm_randomx_program_aarch64_vm_instructions_end_v2
.endif
	.global rxarm_randomx_program_aarch64_vm_instructions_end_light_v1
.set rxarm_randomx_program_aarch64_vm_instructions_end_light_v1, Lrxarm_randomx_program_aarch64_vm_instructions_end_light_v1
.if {macho}
.global _rxarm_randomx_program_aarch64_vm_instructions_end_light_v1
.alt_entry _rxarm_randomx_program_aarch64_vm_instructions_end_light_v1
.alt_entry rxarm_randomx_program_aarch64_vm_instructions_end_light_v1
.set _rxarm_randomx_program_aarch64_vm_instructions_end_light_v1, rxarm_randomx_program_aarch64_vm_instructions_end_light_v1
.endif
	.global rxarm_randomx_program_aarch64_vm_instructions_end_light_v2
.set rxarm_randomx_program_aarch64_vm_instructions_end_light_v2, Lrxarm_randomx_program_aarch64_vm_instructions_end_light_v2
.if {macho}
.global _rxarm_randomx_program_aarch64_vm_instructions_end_light_v2
.alt_entry _rxarm_randomx_program_aarch64_vm_instructions_end_light_v2
.alt_entry rxarm_randomx_program_aarch64_vm_instructions_end_light_v2
.set _rxarm_randomx_program_aarch64_vm_instructions_end_light_v2, rxarm_randomx_program_aarch64_vm_instructions_end_light_v2
.endif
	.global rxarm_randomx_init_dataset_aarch64
.set rxarm_randomx_init_dataset_aarch64, Lrxarm_randomx_init_dataset_aarch64
.if {macho}
.global _rxarm_randomx_init_dataset_aarch64
.alt_entry _rxarm_randomx_init_dataset_aarch64
.alt_entry rxarm_randomx_init_dataset_aarch64
.set _rxarm_randomx_init_dataset_aarch64, rxarm_randomx_init_dataset_aarch64
.endif
	.global rxarm_randomx_init_dataset_aarch64_end
.set rxarm_randomx_init_dataset_aarch64_end, Lrxarm_randomx_init_dataset_aarch64_end
.if {macho}
.global _rxarm_randomx_init_dataset_aarch64_end
.alt_entry _rxarm_randomx_init_dataset_aarch64_end
.alt_entry rxarm_randomx_init_dataset_aarch64_end
.set _rxarm_randomx_init_dataset_aarch64_end, rxarm_randomx_init_dataset_aarch64_end
.endif
	.global rxarm_randomx_calc_dataset_item_aarch64
.set rxarm_randomx_calc_dataset_item_aarch64, Lrxarm_randomx_calc_dataset_item_aarch64
.if {macho}
.global _rxarm_randomx_calc_dataset_item_aarch64
.alt_entry _rxarm_randomx_calc_dataset_item_aarch64
.alt_entry rxarm_randomx_calc_dataset_item_aarch64
.set _rxarm_randomx_calc_dataset_item_aarch64, rxarm_randomx_calc_dataset_item_aarch64
.endif
	.global rxarm_randomx_calc_dataset_item_aarch64_prefetch
.set rxarm_randomx_calc_dataset_item_aarch64_prefetch, Lrxarm_randomx_calc_dataset_item_aarch64_prefetch
.if {macho}
.global _rxarm_randomx_calc_dataset_item_aarch64_prefetch
.alt_entry _rxarm_randomx_calc_dataset_item_aarch64_prefetch
.alt_entry rxarm_randomx_calc_dataset_item_aarch64_prefetch
.set _rxarm_randomx_calc_dataset_item_aarch64_prefetch, rxarm_randomx_calc_dataset_item_aarch64_prefetch
.endif
	.global rxarm_randomx_calc_dataset_item_aarch64_mix
.set rxarm_randomx_calc_dataset_item_aarch64_mix, Lrxarm_randomx_calc_dataset_item_aarch64_mix
.if {macho}
.global _rxarm_randomx_calc_dataset_item_aarch64_mix
.alt_entry _rxarm_randomx_calc_dataset_item_aarch64_mix
.alt_entry rxarm_randomx_calc_dataset_item_aarch64_mix
.set _rxarm_randomx_calc_dataset_item_aarch64_mix, rxarm_randomx_calc_dataset_item_aarch64_mix
.endif
	.global rxarm_randomx_calc_dataset_item_aarch64_store_result
.set rxarm_randomx_calc_dataset_item_aarch64_store_result, Lrxarm_randomx_calc_dataset_item_aarch64_store_result
.if {macho}
.global _rxarm_randomx_calc_dataset_item_aarch64_store_result
.alt_entry _rxarm_randomx_calc_dataset_item_aarch64_store_result
.alt_entry rxarm_randomx_calc_dataset_item_aarch64_store_result
.set _rxarm_randomx_calc_dataset_item_aarch64_store_result, rxarm_randomx_calc_dataset_item_aarch64_store_result
.endif
	.global rxarm_randomx_calc_dataset_item_aarch64_end
.set rxarm_randomx_calc_dataset_item_aarch64_end, Lrxarm_randomx_calc_dataset_item_aarch64_end
.if {macho}
.global _rxarm_randomx_calc_dataset_item_aarch64_end
.alt_entry _rxarm_randomx_calc_dataset_item_aarch64_end
.alt_entry rxarm_randomx_calc_dataset_item_aarch64_end
.set _rxarm_randomx_calc_dataset_item_aarch64_end, rxarm_randomx_calc_dataset_item_aarch64_end
.endif





	.balign 4
Lrxarm_randomx_program_aarch64:
	sub	sp, sp, 192
	stp	x16, x17, [sp]
	str	x19, [sp, 16]
	stp	x20, x21, [sp, 32]
	stp	x22, x23, [sp, 48]
	stp	x24, x25, [sp, 64]
	stp	x26, x27, [sp, 80]
	stp	x28, x29, [sp, 96]
	stp	x8, x30, [sp, 112]
	stp	d8, d9, [sp, 128]
	stp	d10, d11, [sp, 144]
	stp	d12, d13, [sp, 160]
	stp	d14, d15, [sp, 176]

	mov	x4, xzr
	mov	x5, xzr
	mov	x6, xzr
	mov	x7, xzr
	mov	x12, xzr
	mov	x13, xzr
	mov	x14, xzr
	mov	x15, xzr

	ldp	x9, x1, [x1]

	mov	x10, x9

	ldp	q24, q25, [x0, 192]
	ldp	q26, q27, [x0, 224]

	mov	x16, 0x00FFFFFFFFC00000
	ins	v29.d[0], x16
	ins	v29.d[1], x16

	ldr	q30, [x0, 64]

	mov	x16, 0x80f0000000000000
	ins	v31.d[0], x16
	ins	v31.d[1], x16

	mrs	x8, fpcr
	rbit	x8, x8

	str	x0, [sp, -112]!

	ldr	x0, Lliteral_x0
	ldr	x11, Lliteral_x11
	ldr	x21, Lliteral_x21
	ldr	x22, Lliteral_x22
	ldr	x23, Lliteral_x23
	ldr	x24, Lliteral_x24
	ldr	x25, Lliteral_x25
	ldr	x26, Lliteral_x26
	ldr	x27, Lliteral_x27
	ldr	x28, Lliteral_x28
	ldr	x29, Lliteral_x29
	ldr	x30, Lliteral_x30

	ldr	q0, Lliteral_v0
	ldr	q1, Lliteral_v1
	ldr	q2, Lliteral_v2
	ldr	q3, Lliteral_v3
	ldr	q4, Lliteral_v4
	ldr	q5, Lliteral_v5
	ldr	q6, Lliteral_v6
	ldr	q7, Lliteral_v7
	ldr	q8, Lliteral_v8
	ldr	q9, Lliteral_v9
	ldr	q10, Lliteral_v10
	ldr	q11, Lliteral_v11
	ldr	q12, Lliteral_v12
	ldr	q13, Lliteral_v13
	ldr	q14, Lliteral_v14
	ldr	q15, Lliteral_v15

Lrxarm_randomx_program_aarch64_main_loop:
	lsr	x20, x10, 32

	and	w16, w10, 1
	and	w17, w20, 1

	add	x16, x16, x2
	add	x17, x17, x2

	ldp	x20, x19, [x16]
	eor	x4, x4, x20
	eor	x5, x5, x19
	ldp	x20, x19, [x16, 16]
	eor	x6, x6, x20
	eor	x7, x7, x19
	ldp	x20, x19, [x16, 32]
	eor	x12, x12, x20
	eor	x13, x13, x19
	ldp	x20, x19, [x16, 48]
	eor	x14, x14, x20
	eor	x15, x15, x19

	ldpsw	x20, x19, [x17]
	ins	v16.d[0], x20
	ins	v16.d[1], x19
	ldpsw	x20, x19, [x17, 8]
	ins	v17.d[0], x20
	ins	v17.d[1], x19
	ldpsw	x20, x19, [x17, 16]
	ins	v18.d[0], x20
	ins	v18.d[1], x19
	ldpsw	x20, x19, [x17, 24]
	ins	v19.d[0], x20
	ins	v19.d[1], x19
	scvtf	v16.2d, v16.2d
	scvtf	v17.2d, v17.2d
	scvtf	v18.2d, v18.2d
	scvtf	v19.2d, v19.2d

	ldpsw	x20, x19, [x17, 32]
	ins	v20.d[0], x20
	ins	v20.d[1], x19
	ldpsw	x20, x19, [x17, 40]
	ins	v21.d[0], x20
	ins	v21.d[1], x19
	ldpsw	x20, x19, [x17, 48]
	ins	v22.d[0], x20
	ins	v22.d[1], x19
	ldpsw	x20, x19, [x17, 56]
	ins	v23.d[0], x20
	ins	v23.d[1], x19
	scvtf	v20.2d, v20.2d
	scvtf	v21.2d, v21.2d
	scvtf	v22.2d, v22.2d
	scvtf	v23.2d, v23.2d
	bif	v20.16b, v30.16b, v29.16b
	bif	v21.16b, v30.16b, v29.16b
	bif	v22.16b, v30.16b, v29.16b
	bif	v23.16b, v30.16b, v29.16b

Lrxarm_randomx_program_aarch64_vm_instructions:

	.fill 384*12,4,0

Lliteral_x0:  .fill 1,8,0
Lliteral_x11: .fill 1,8,0
Lliteral_x21: .fill 1,8,0
Lliteral_x22: .fill 1,8,0
Lliteral_x23: .fill 1,8,0
Lliteral_x24: .fill 1,8,0
Lliteral_x25: .fill 1,8,0
Lliteral_x26: .fill 1,8,0
Lliteral_x27: .fill 1,8,0
Lliteral_x28: .fill 1,8,0
Lliteral_x29: .fill 1,8,0
Lliteral_x30: .fill 1,8,0
Lrxarm_randomx_program_aarch64_imul_rcp_literals_end:

Lliteral_v0:  .fill 2,8,0
Lliteral_v1:  .fill 2,8,0
Lliteral_v2:  .fill 2,8,0
Lliteral_v3:  .fill 2,8,0
Lliteral_v4:  .fill 2,8,0
Lliteral_v5:  .fill 2,8,0
Lliteral_v6:  .fill 2,8,0
Lliteral_v7:  .fill 2,8,0
Lliteral_v8:  .fill 2,8,0
Lliteral_v9:  .fill 2,8,0
Lliteral_v10: .fill 2,8,0
Lliteral_v11: .fill 2,8,0
Lliteral_v12: .fill 2,8,0
Lliteral_v13: .fill 2,8,0
Lliteral_v14: .fill 2,8,0
Lliteral_v15: .fill 2,8,0

Lrxarm_randomx_program_aarch64_vm_instructions_end:
	lsr	x10, x9, 32

	eor	x9, x9, x20

	mov	w20, w9

	ror	x9, x9, 32

Lrxarm_randomx_program_aarch64_cacheline_align_mask1:
	and	x20, x20, 1
	add	x20, x20, x1

	prfm	pldl2strm, [x20]

Lrxarm_randomx_program_aarch64_cacheline_align_mask2:
	and	x10, x10, 1
	add	x10, x10, x1

Lrxarm_randomx_program_aarch64_xor_with_dataset_line:
Lrx_program_xor_with_dataset_line:
	ldp	x20, x19, [x10]
	eor	x4, x4, x20
	eor	x5, x5, x19
	ldp	x20, x19, [x10, 16]
	eor	x6, x6, x20
	eor	x7, x7, x19
	ldp	x20, x19, [x10, 32]
	eor	x12, x12, x20
	eor	x13, x13, x19
	ldp	x20, x19, [x10, 48]
	eor	x14, x14, x20
	eor	x15, x15, x19

Lrxarm_randomx_program_aarch64_update_spMix1:
	eor	x10, x0, x0

	stp	x4, x5, [x17, 0]
	stp	x6, x7, [x17, 16]
	stp	x12, x13, [x17, 32]
	stp	x14, x15, [x17, 48]

Lrxarm_randomx_program_aarch64_v2_FE_mix:

	b Lrxarm_randomx_program_aarch64_v1_FE_mix


	aese	v16.16b, v28.16b
	aesd	v17.16b, v28.16b
	aese	v18.16b, v28.16b
	aesd	v19.16b, v28.16b

	aesmc	v16.16b, v16.16b
	aesimc	v17.16b, v17.16b
	aesmc	v18.16b, v18.16b
	aesimc	v19.16b, v19.16b

	eor	v16.16b, v16.16b, v20.16b
	eor	v17.16b, v17.16b, v20.16b
	eor	v18.16b, v18.16b, v20.16b
	eor	v19.16b, v19.16b, v20.16b


	aese	v16.16b, v28.16b
	aesd	v17.16b, v28.16b
	aese	v18.16b, v28.16b
	aesd	v19.16b, v28.16b

	aesmc	v16.16b, v16.16b
	aesimc	v17.16b, v17.16b
	aesmc	v18.16b, v18.16b
	aesimc	v19.16b, v19.16b

	eor	v16.16b, v16.16b, v21.16b
	eor	v17.16b, v17.16b, v21.16b
	eor	v18.16b, v18.16b, v21.16b
	eor	v19.16b, v19.16b, v21.16b


	aese	v16.16b, v28.16b
	aesd	v17.16b, v28.16b
	aese	v18.16b, v28.16b
	aesd	v19.16b, v28.16b

	aesmc	v16.16b, v16.16b
	aesimc	v17.16b, v17.16b
	aesmc	v18.16b, v18.16b
	aesimc	v19.16b, v19.16b

	eor	v16.16b, v16.16b, v22.16b
	eor	v17.16b, v17.16b, v22.16b
	eor	v18.16b, v18.16b, v22.16b
	eor	v19.16b, v19.16b, v22.16b


	aese	v16.16b, v28.16b
	aesd	v17.16b, v28.16b
	aese	v18.16b, v28.16b
	aesd	v19.16b, v28.16b

	aesmc	v16.16b, v16.16b
	aesimc	v17.16b, v17.16b
	aesmc	v18.16b, v18.16b
	aesimc	v19.16b, v19.16b

	eor	v16.16b, v16.16b, v23.16b
	eor	v17.16b, v17.16b, v23.16b
	eor	v18.16b, v18.16b, v23.16b
	eor	v19.16b, v19.16b, v23.16b

	b Lrandomx_program_aarch64_FE_store

Lrxarm_randomx_program_aarch64_v1_FE_mix:
	eor	v16.16b, v16.16b, v20.16b
	eor	v17.16b, v17.16b, v21.16b
	eor	v18.16b, v18.16b, v22.16b
	eor	v19.16b, v19.16b, v23.16b

Lrandomx_program_aarch64_FE_store:

	stp	q16, q17, [x16, 0]
	stp	q18, q19, [x16, 32]

	subs	x3, x3, 1
	bne	Lrxarm_randomx_program_aarch64_main_loop

	ldr	x0, [sp], 112

	stp	x4, x5, [x0, 0]
	stp	x6, x7, [x0, 16]
	stp	x12, x13, [x0, 32]
	stp	x14, x15, [x0, 48]

	stp	q16, q17, [x0, 64]
	stp	q18, q19, [x0, 96]
	stp	q20, q21, [x0, 128]
	stp	q22, q23, [x0, 160]

	ldp	x16, x17, [sp]
	ldr	x19, [sp, 16]
	ldp	x20, x21, [sp, 32]
	ldp	x22, x23, [sp, 48]
	ldp	x24, x25, [sp, 64]
	ldp	x26, x27, [sp, 80]
	ldp	x28, x29, [sp, 96]
	ldp	x8, x30, [sp, 112]
	ldp	d8, d9, [sp, 128]
	ldp	d10, d11, [sp, 144]
	ldp	d12, d13, [sp, 160]
	ldp	d14, d15, [sp, 176]
	add	sp, sp, 192

	ret

Lrxarm_randomx_program_aarch64_vm_instructions_end_light:
	stp	x0, x1, [sp, 80]
	stp	x2, x30, [sp, 96]

	lsr	x2, x9, 32

Lrxarm_randomx_program_aarch64_light_cacheline_align_mask:
	and	w2, w2, 1

Lrxarm_randomx_program_aarch64_vm_instructions_end_light_tweak:
	eor	x9, x9, x20

	ror	x9, x9, 32

	mov	x0, x1

	add	x1, sp, 16

	lsr	x2, x2, 6

Lrxarm_randomx_program_aarch64_light_dataset_offset:
	add	x2, x2, 0
	add	x2, x2, 0

	bl	Lrx_calc_dataset_item

	add	x10, sp, 16
	ldp	x0, x1, [sp, 80]
	ldp	x2, x30, [sp, 96]

	b	Lrx_program_xor_with_dataset_line

Lrxarm_randomx_program_aarch64_vm_instructions_end_v1:
	lsr	x10, x9, 32
	eor	x9, x9, x20
	mov	w20, w9
	ror	x9, x9, 32

Lrxarm_randomx_program_aarch64_vm_instructions_end_v2:
	lsr	x10, x9, 32
	ror	x9, x9, 32
	eor	x9, x9, x20
	mov	w20, w9

Lrxarm_randomx_program_aarch64_vm_instructions_end_light_v1:
	eor	x9, x9, x20
	ror	x9, x9, 32

Lrxarm_randomx_program_aarch64_vm_instructions_end_light_v2:
	ror	x9, x9, 32
	eor	x9, x9, x20

Lrxarm_randomx_program_aarch64_v2_FE_mix_soft_aes:
	sub	sp, sp, 176

	stp	x0, x1, [sp]
	stp	x2, x3, [sp, 16]
	stp	x4, x5, [sp, 32]
	stp	x6, x7, [sp, 48]
	stp	x8, x9, [sp, 64]
	stp	x10, x11, [sp, 80]
	stp	x12, x13, [sp, 96]
	stp	x14, x15, [sp, 112]
	stp	x16, x30, [sp, 128]
	stp	q0, q1, [sp, 144]

	adr	x19, Lrxarm_randomx_program_aarch64_aes_lut_pointers
	ldp	x19, x20, [x19]

	mov	v0.16b, v16.16b
	mov	v1.16b, v20.16b
	bl	Lrandomx_soft_aesenc
	mov	v1.16b, v21.16b
	bl	Lrandomx_soft_aesenc
	mov	v1.16b, v22.16b
	bl	Lrandomx_soft_aesenc
	mov	v1.16b, v23.16b
	bl	Lrandomx_soft_aesenc
	mov	v16.16b, v0.16b

	mov	v0.16b, v17.16b
	mov	v1.16b, v20.16b
	bl	Lrandomx_soft_aesdec
	mov	v1.16b, v21.16b
	bl	Lrandomx_soft_aesdec
	mov	v1.16b, v22.16b
	bl	Lrandomx_soft_aesdec
	mov	v1.16b, v23.16b
	bl	Lrandomx_soft_aesdec
	mov	v17.16b, v0.16b

	mov	v0.16b, v18.16b
	mov	v1.16b, v20.16b
	bl	Lrandomx_soft_aesenc
	mov	v1.16b, v21.16b
	bl	Lrandomx_soft_aesenc
	mov	v1.16b, v22.16b
	bl	Lrandomx_soft_aesenc
	mov	v1.16b, v23.16b
	bl	Lrandomx_soft_aesenc
	mov	v18.16b, v0.16b

	mov	v0.16b, v19.16b
	mov	v1.16b, v20.16b
	bl	Lrandomx_soft_aesdec
	mov	v1.16b, v21.16b
	bl	Lrandomx_soft_aesdec
	mov	v1.16b, v22.16b
	bl	Lrandomx_soft_aesdec
	mov	v1.16b, v23.16b
	bl	Lrandomx_soft_aesdec
	mov	v19.16b, v0.16b

	ldp	x0, x1, [sp]
	ldp	x2, x3, [sp, 16]
	ldp	x4, x5, [sp, 32]
	ldp	x6, x7, [sp, 48]
	ldp	x8, x9, [sp, 64]
	ldp	x10, x11, [sp, 80]
	ldp	x12, x13, [sp, 96]
	ldp	x14, x15, [sp, 112]
	ldp	x16, x30, [sp, 128]
	ldp	q0, q1, [sp, 144]

	add	sp, sp, 176

	b Lrandomx_program_aarch64_FE_store


Lrandomx_soft_aesenc:
	umov	w4, v0.b[5]
	umov	w1, v0.b[10]
	umov	w12, v0.b[15]
	umov	w9, v0.b[9]
	umov	w2, v0.b[14]
	umov	w11, v0.b[3]
	umov	w5, v0.b[0]
	umov	w16, v0.b[4]
	add	x4, x4, 256
	add	x1, x1, 512
	add	x12, x12, 768
	umov	w3, v0.b[13]
	umov	w8, v0.b[2]
	umov	w7, v0.b[7]
	add	x9, x9, 256
	add	x2, x2, 512
	add	x11, x11, 768
	ldr	w10, [x19, x4, lsl 2]
	ldr	w15, [x19, x5, lsl 2]
	umov	w13, v0.b[8]
	ldr	w14, [x19, x12, lsl 2]
	umov	w6, v0.b[1]
	ldr	w1, [x19, x1, lsl 2]
	eor	w10, w10, w15
	ldr	w2, [x19, x2, lsl 2]
	umov	w5, v0.b[6]
	ldr	w9, [x19, x9, lsl 2]
	umov	w4, v0.b[11]
	ldr	w12, [x19, x16, lsl 2]
	eor	w1, w1, w14
	ldr	w11, [x19, x11, lsl 2]
	eor	w1, w1, w10
	add	x8, x8, 512
	add	x3, x3, 256
	add	x7, x7, 768
	eor	w9, w9, w12
	fmov	s28, w1
	eor	w1, w2, w11
	umov	w10, v0.b[12]
	eor	w1, w1, w9
	ldr	w3, [x19, x3, lsl 2]
	add	x6, x6, 256
	ldr	w9, [x19, x13, lsl 2]
	ins	v28.s[1], w1
	ldr	w2, [x19, x8, lsl 2]
	add	x5, x5, 512
	ldr	w7, [x19, x7, lsl 2]
	add	x4, x4, 768
	eor	w1, w3, w9
	ldr	w3, [x19, x6, lsl 2]
	eor	w2, w2, w7
	ldr	w6, [x19, x10, lsl 2]
	eor	w2, w2, w1
	ldr	w1, [x19, x5, lsl 2]
	ldr	w0, [x19, x4, lsl 2]
	eor	w3, w3, w6
	ins	v28.s[2], w2
	eor	w0, w1, w0
	eor	w0, w0, w3
	ins	v28.s[3], w0
	eor	v0.16b, v1.16b, v28.16b
	ret

Lrandomx_soft_aesdec:
	umov	w1, v0.b[10]
	umov	w3, v0.b[7]
	umov	w12, v0.b[13]
	umov	w2, v0.b[14]
	umov	w9, v0.b[11]
	umov	w11, v0.b[1]
	umov	w4, v0.b[0]
	umov	w16, v0.b[4]
	add	x3, x3, 768
	add	x1, x1, 512
	add	x12, x12, 256
	umov	w8, v0.b[5]
	umov	w6, v0.b[2]
	umov	w7, v0.b[15]
	add	x9, x9, 768
	add	x2, x2, 512
	add	x11, x11, 256
	ldr	w15, [x20, x3, lsl 2]
	ldr	w10, [x20, x4, lsl 2]
	umov	w13, v0.b[8]
	ldr	w14, [x20, x12, lsl 2]
	umov	w5, v0.b[9]
	ldr	w1, [x20, x1, lsl 2]
	umov	w3, v0.b[6]
	ldr	w12, [x20, x9, lsl 2]
	umov	w4, v0.b[3]
	ldr	w9, [x20, x16, lsl 2]
	eor	w1, w1, w15
	ldr	w2, [x20, x2, lsl 2]
	eor	w10, w10, w14
	ldr	w11, [x20, x11, lsl 2]
	eor	w1, w1, w10
	add	x8, x8, 256
	add	x6, x6, 512
	add	x7, x7, 768
	eor	w2, w2, w12
	fmov	s28, w1
	eor	w1, w9, w11
	eor	w1, w2, w1
	umov	w9, v0.b[12]
	ldr	w2, [x20, x13, lsl 2]
	add	x5, x5, 256
	ldr	w8, [x20, x8, lsl 2]
	ins	v28.s[1], w1
	ldr	w6, [x20, x6, lsl 2]
	add	x3, x3, 512
	ldr	w7, [x20, x7, lsl 2]
	add	x4, x4, 768
	eor	w2, w2, w8
	ldr	w1, [x20, x9, lsl 2]
	eor	w6, w6, w7
	ldr	w3, [x20, x3, lsl 2]
	eor	w2, w2, w6
	ldr	w4, [x20, x4, lsl 2]
	ldr	w5, [x20, x5, lsl 2]
	ins	v28.s[2], w2
	eor	w0, w1, w5
	eor	w1, w3, w4
	eor	w0, w0, w1
	ins	v28.s[3], w0
	eor	v0.16b, v1.16b, v28.16b
	ret

Lrxarm_randomx_program_aarch64_aes_lut_pointers:
	.fill 2, 8, 0



Lrxarm_randomx_init_dataset_aarch64:
	stp	x20, x30, [sp, -16]!

	ldr	x0, [x0]

Lrxarm_randomx_init_dataset_aarch64_main_loop:
	bl	Lrx_calc_dataset_item
	add	x1, x1, 64
	add	x2, x2, 1
	cmp	x2, x3
	bne	Lrxarm_randomx_init_dataset_aarch64_main_loop

	ldp	x20, x30, [sp], 16

	ret

Lrxarm_randomx_init_dataset_aarch64_end:


Lrxarm_randomx_calc_dataset_item_aarch64:
Lrx_calc_dataset_item:
	sub	sp, sp, 112
	stp	x0, x1, [sp]
	stp	x2, x3, [sp, 16]
	stp	x4, x5, [sp, 32]
	stp	x6, x7, [sp, 48]
	stp	x8, x9, [sp, 64]
	stp	x10, x11, [sp, 80]
	stp	x12, x13, [sp, 96]

	ldr	x12, LsuperscalarMul0

	mov	x8, x0
	mov	x9, x1
	mov	x10, x2

	madd	x0, x2, x12, x12

	ldr	x12, LsuperscalarAdd1
	eor	x1, x0, x12

	ldr	x12, LsuperscalarAdd2
	eor	x2, x0, x12

	ldr	x12, LsuperscalarAdd3
	eor	x3, x0, x12

	ldr	x12, LsuperscalarAdd4
	eor	x4, x0, x12

	ldr	x12, LsuperscalarAdd5
	eor	x5, x0, x12

	ldr	x12, LsuperscalarAdd6
	eor	x6, x0, x12

	ldr	x12, LsuperscalarAdd7
	eor	x7, x0, x12

	b	Lrx_calc_dataset_item_prefetch

LsuperscalarMul0: .quad 6364136223846793005
LsuperscalarAdd1: .quad 9298411001130361340
LsuperscalarAdd2: .quad 12065312585734608966
LsuperscalarAdd3: .quad 9306329213124626780
LsuperscalarAdd4: .quad 5281919268842080866
LsuperscalarAdd5: .quad 10536153434571861004
LsuperscalarAdd6: .quad 3398623926847679864
LsuperscalarAdd7: .quad 9549104520008361294


Lrxarm_randomx_calc_dataset_item_aarch64_prefetch:
Lrx_calc_dataset_item_prefetch:
	and	x11, x10, 1
	add	x11, x8, x11, lsl 6
	prfm	pldl2strm, [x11]


Lrxarm_randomx_calc_dataset_item_aarch64_mix:
	ldp	x12, x13, [x11]
	eor	x0, x0, x12
	eor	x1, x1, x13
	ldp	x12, x13, [x11, 16]
	eor	x2, x2, x12
	eor	x3, x3, x13
	ldp	x12, x13, [x11, 32]
	eor	x4, x4, x12
	eor	x5, x5, x13
	ldp	x12, x13, [x11, 48]
	eor	x6, x6, x12
	eor	x7, x7, x13

Lrxarm_randomx_calc_dataset_item_aarch64_store_result:
	stp	x0, x1, [x9]
	stp	x2, x3, [x9, 16]
	stp	x4, x5, [x9, 32]
	stp	x6, x7, [x9, 48]

	ldp	x0, x1, [sp]
	ldp	x2, x3, [sp, 16]
	ldp	x4, x5, [sp, 32]
	ldp	x6, x7, [sp, 48]
	ldp	x8, x9, [sp, 64]
	ldp	x10, x11, [sp, 80]
	ldp	x12, x13, [sp, 96]
	add	sp, sp, 112

	ret

Lrxarm_randomx_calc_dataset_item_aarch64_end:


"#, macho = const cfg!(target_vendor = "apple") as u8);

unsafe extern "C" {
    static rxarm_randomx_program_aarch64: u8;
    static rxarm_randomx_program_aarch64_main_loop: u8;
    static rxarm_randomx_program_aarch64_vm_instructions: u8;
    static rxarm_randomx_program_aarch64_imul_rcp_literals_end: u8;
    static rxarm_randomx_program_aarch64_vm_instructions_end: u8;
    static rxarm_randomx_program_aarch64_cacheline_align_mask1: u8;
    static rxarm_randomx_program_aarch64_cacheline_align_mask2: u8;
    static rxarm_randomx_program_aarch64_update_spMix1: u8;
    static rxarm_randomx_program_aarch64_v2_FE_mix: u8;
    static rxarm_randomx_program_aarch64_v1_FE_mix: u8;
    static rxarm_randomx_program_aarch64_v2_FE_mix_soft_aes: u8;
    static rxarm_randomx_program_aarch64_aes_lut_pointers: u8;
    static rxarm_randomx_program_aarch64_vm_instructions_end_light: u8;
    static rxarm_randomx_program_aarch64_vm_instructions_end_light_tweak: u8;
    static rxarm_randomx_program_aarch64_light_cacheline_align_mask: u8;
    static rxarm_randomx_program_aarch64_light_dataset_offset: u8;
    static rxarm_randomx_program_aarch64_vm_instructions_end_v1: u8;
    static rxarm_randomx_program_aarch64_vm_instructions_end_v2: u8;
    static rxarm_randomx_program_aarch64_vm_instructions_end_light_v1: u8;
    static rxarm_randomx_program_aarch64_vm_instructions_end_light_v2: u8;
    static rxarm_randomx_init_dataset_aarch64: u8;
    static rxarm_randomx_init_dataset_aarch64_end: u8;
    static rxarm_randomx_calc_dataset_item_aarch64: u8;
    static rxarm_randomx_calc_dataset_item_aarch64_prefetch: u8;
    static rxarm_randomx_calc_dataset_item_aarch64_mix: u8;
    static rxarm_randomx_calc_dataset_item_aarch64_store_result: u8;
    static rxarm_randomx_calc_dataset_item_aarch64_end: u8;
}
pub(super) fn program_aarch64_main_loop() -> usize {
    (&raw const rxarm_randomx_program_aarch64_main_loop).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_vm_instructions() -> usize {
    (&raw const rxarm_randomx_program_aarch64_vm_instructions).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_imul_rcp_literals_end() -> usize {
    (&raw const rxarm_randomx_program_aarch64_imul_rcp_literals_end).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_vm_instructions_end() -> usize {
    (&raw const rxarm_randomx_program_aarch64_vm_instructions_end).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_cacheline_align_mask1() -> usize {
    (&raw const rxarm_randomx_program_aarch64_cacheline_align_mask1).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_cacheline_align_mask2() -> usize {
    (&raw const rxarm_randomx_program_aarch64_cacheline_align_mask2).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_update_sp_mix1() -> usize {
    (&raw const rxarm_randomx_program_aarch64_update_spMix1).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_v2_fe_mix() -> usize {
    (&raw const rxarm_randomx_program_aarch64_v2_FE_mix).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_v1_fe_mix() -> usize {
    (&raw const rxarm_randomx_program_aarch64_v1_FE_mix).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_v2_fe_mix_soft_aes() -> usize {
    (&raw const rxarm_randomx_program_aarch64_v2_FE_mix_soft_aes).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_aes_lut_pointers() -> usize {
    (&raw const rxarm_randomx_program_aarch64_aes_lut_pointers).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_vm_instructions_end_light() -> usize {
    (&raw const rxarm_randomx_program_aarch64_vm_instructions_end_light).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_vm_instructions_end_light_tweak() -> usize {
    (&raw const rxarm_randomx_program_aarch64_vm_instructions_end_light_tweak).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_light_cacheline_align_mask() -> usize {
    (&raw const rxarm_randomx_program_aarch64_light_cacheline_align_mask).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_light_dataset_offset() -> usize {
    (&raw const rxarm_randomx_program_aarch64_light_dataset_offset).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_vm_instructions_end_v1() -> usize {
    (&raw const rxarm_randomx_program_aarch64_vm_instructions_end_v1).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_vm_instructions_end_v2() -> usize {
    (&raw const rxarm_randomx_program_aarch64_vm_instructions_end_v2).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_vm_instructions_end_light_v1() -> usize {
    (&raw const rxarm_randomx_program_aarch64_vm_instructions_end_light_v1).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn program_aarch64_vm_instructions_end_light_v2() -> usize {
    (&raw const rxarm_randomx_program_aarch64_vm_instructions_end_light_v2).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn init_dataset_aarch64() -> usize {
    (&raw const rxarm_randomx_init_dataset_aarch64).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn init_dataset_aarch64_end() -> usize {
    (&raw const rxarm_randomx_init_dataset_aarch64_end).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn calc_dataset_item_aarch64() -> usize {
    (&raw const rxarm_randomx_calc_dataset_item_aarch64).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn calc_dataset_item_aarch64_prefetch() -> usize {
    (&raw const rxarm_randomx_calc_dataset_item_aarch64_prefetch).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn calc_dataset_item_aarch64_mix() -> usize {
    (&raw const rxarm_randomx_calc_dataset_item_aarch64_mix).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn calc_dataset_item_aarch64_store_result() -> usize {
    (&raw const rxarm_randomx_calc_dataset_item_aarch64_store_result).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn calc_dataset_item_aarch64_end() -> usize {
    (&raw const rxarm_randomx_calc_dataset_item_aarch64_end).addr()
        - (&raw const rxarm_randomx_program_aarch64).addr()
}
pub(super) fn part(start: usize, end: usize) -> &'static [u8] {
    assert!(start <= end && end <= calc_dataset_item_aarch64_end());
    // SAFETY: both offsets delimit the contiguous immutable assembler template.
    unsafe {
        std::slice::from_raw_parts(
            (&raw const rxarm_randomx_program_aarch64).add(start),
            end - start,
        )
    }
}
pub(super) fn program() -> &'static [u8] {
    part(0, init_dataset_aarch64_end())
}
