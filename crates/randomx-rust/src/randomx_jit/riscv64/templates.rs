// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2021, SChernykh <https://github.com/SChernykh>
// Copyright (c) 2026 RxMiner contributors
// Rust integrated RV64GC templates. See ../../randomx_core/LICENSE.
core::arch::global_asm!(
    r#"
.section .rodata
.option rvc
.option norelax

.global rxrv_randomx_riscv64_literals
.global rxrv_randomx_riscv64_literals_end
.global rxrv_randomx_riscv64_data_init
.global rxrv_randomx_riscv64_fix_data_call
.global rxrv_randomx_riscv64_prologue
.global rxrv_randomx_riscv64_loop_begin
.global rxrv_randomx_riscv64_data_read
.global rxrv_randomx_riscv64_data_read_v2_tweak
.global rxrv_randomx_riscv64_data_read_light
.global rxrv_randomx_riscv64_data_read_light_v1
.global rxrv_randomx_riscv64_data_read_light_v2
.global rxrv_randomx_riscv64_fix_loop_call
.global rxrv_randomx_riscv64_spad_store
.global rxrv_randomx_riscv64_spad_store_softaes
.global rxrv_randomx_riscv64_loop_end
.global rxrv_randomx_riscv64_fix_continue_loop
.global rxrv_randomx_riscv64_epilogue
.global rxrv_randomx_riscv64_softaes
.global rxrv_randomx_riscv64_program_end
.global rxrv_randomx_riscv64_ssh_init
.global rxrv_randomx_riscv64_ssh_load
.global rxrv_randomx_riscv64_ssh_prefetch
.global rxrv_randomx_riscv64_ssh_end
rxrv_randomx_riscv64_literals:
literal_pool:
    .dword 6364136223846793005
    .dword 9298411001130361340
    .dword 12065312585734608966
    .dword 9306329213124626780
    .dword 5281919268842080866
    .dword 10536153434571861004
    .dword 3398623926847679864
    .dword 9549104520008361294
    .word 0x00000000
    .word 0x00000002
    .word 0x00000003
    .word 0x00000001
    .word (16384 -8)
    .word (262144 -8)
    .word (2097152 -64)
    .word (2147483648 -64)
    .dword 0x80f0000000000000
    .dword 0x00ffffffffffffff
rxrv_randomx_riscv64_literals_end:
    .dword 0
    .dword 0
    .dword 0
    .dword 0
    .fill 238,8,0
rxrv_randomx_riscv64_data_init:
    addi sp, sp, -32
    mv x5, x11
    ld x6, 0(x10)
    sd x1, 0(sp)
    sd x3, 8(sp)
    lla x3, literal_pool
    sd x8, 16(sp)
    sd x9, 24(sp)
    mv x16, x12
    mv x17, x13
init_item:
    mv x7, x16
rxrv_randomx_riscv64_fix_data_call:
    jal superscalar_hash
    sd x8, 0(x5)
    sd x9, 8(x5)
    sd x10, 16(x5)
    sd x11, 24(x5)
    sd x12, 32(x5)
    sd x13, 40(x5)
    sd x14, 48(x5)
    sd x15, 56(x5)
    addi x5, x5, 64
    addi x16, x16, 1
    bltu x16, x17, init_item
    ld x1, 0(sp)
    ld x3, 8(sp)
    ld x8, 16(sp)
    ld x9, 24(sp)
    addi sp, sp, 32
    ret
rxrv_randomx_riscv64_prologue:
    addi sp, sp, -224
    mv x5, x12
    sd x10, 8(sp)
    sd x3, 16(sp)
    sd x8, 32(sp)
    sd x9, 40(sp)
    sd x18, 48(sp)
    sd x19, 56(sp)
    sd x20, 64(sp)
    sd x21, 72(sp)
    sd x22, 80(sp)
    sd x23, 88(sp)
    sd x24, 96(sp)
    sd x25, 104(sp)
    sd x26, 112(sp)
    sd x27, 120(sp)
    fsd f8, 128(sp)
    fsd f9, 136(sp)
    fsd f18, 144(sp)
    fsd f19, 152(sp)
    fsd f20, 160(sp)
    fsd f21, 168(sp)
    fsd f22, 176(sp)
    fsd f23, 184(sp)
    fsd f24, 192(sp)
    fsd f25, 200(sp)
    fsd f26, 208(sp)
    fsd f27, 216(sp)
    mv x24, x13
    sd x1, 0(sp)
    lla x3, literal_pool
    ld x25, 0(x11)
    ld x6, 8(x11)
    lwu x1, 92(x3)
    li x16, 0
    fld f16, 192(x10)
    li x17, 0
    fld f17, 200(x10)
    srli x7, x25, 32
    li x18, 0
    fld f18, 208(x10)
    mv x27, x7
    li x19, 0
    fld f19, 216(x10)
    and x7, x7, x1
    add x7, x7, x6
    li x20, 0
    fld f20, 224(x10)
    li x21, 0
    fld f21, 232(x10)
    li x22, 0
    fld f22, 240(x10)
    li x23, 0
    fld f23, 248(x10)
    lwu x1, 88(x3)
    lwu x10, 80(x3)
    lwu x11, 84(x3)
    and x26, x25, x1
    and x27, x27, x1
    add x26, x26, x5
    add x27, x27, x5
    addi x1, x1, 56
    ld x12, 96(x3)
    ld x13, 104(x3)
    ld x14, 112(x3)
    ld x15, 120(x3)
    fld f26, 176(x3)
    fld f27, 184(x3)
    fld f28, 192(x3)
    fld f29, 200(x3)
    fld f30, 208(x3)
    fld f31, 216(x3)
.balign 4
rxrv_randomx_riscv64_loop_begin:
loop_begin:
    ld x8, 0(x26)
    ld x9, 8(x26)
    ld x30, 16(x26)
    ld x31, 24(x26)
    xor x16, x16, x8
    ld x8, 32(x26)
    xor x17, x17, x9
    ld x9, 40(x26)
    xor x18, x18, x30
    ld x30, 48(x26)
    xor x19, x19, x31
    ld x31, 56(x26)
    xor x20, x20, x8
    lw x8, 0(x27)
    xor x21, x21, x9
    lw x9, 4(x27)
    xor x22, x22, x30
    lw x30, 8(x27)
    xor x23, x23, x31
    lw x31, 12(x27)
    fcvt.d.w f0, x8
    lw x8, 16(x27)
    fcvt.d.w f1, x9
    lw x9, 20(x27)
    fcvt.d.w f2, x30
    lw x30, 24(x27)
    fcvt.d.w f3, x31
    lw x31, 28(x27)
    fcvt.d.w f4, x8
    lw x8, 32(x27)
    fcvt.d.w f5, x9
    lw x9, 36(x27)
    fcvt.d.w f6, x30
    lw x30, 40(x27)
    fcvt.d.w f7, x31
    lw x31, 44(x27)
    fcvt.d.w f8, x8
    lw x8, 48(x27)
    fcvt.d.w f9, x9
    lw x9, 52(x27)
    fcvt.d.w f10, x30
    lw x30, 56(x27)
    fcvt.d.w f11, x31
    lw x31, 60(x27)
    fcvt.d.w f12, x8
    fmv.x.d x8, f8
    fcvt.d.w f13, x9
    fmv.x.d x9, f9
    fcvt.d.w f14, x30
    fmv.x.d x30, f10
    fcvt.d.w f15, x31
    fmv.x.d x31, f11
    and x8, x8, x13
    and x9, x9, x13
    or x8, x8, x14
    or x9, x9, x15
    and x30, x30, x13
    and x31, x31, x13
    or x30, x30, x14
    or x31, x31, x15
    fmv.d.x f8, x8
    fmv.d.x f9, x9
    fmv.d.x f10, x30
    fmv.d.x f11, x31
    fmv.x.d x8, f12
    fmv.x.d x9, f13
    fmv.x.d x30, f14
    fmv.x.d x31, f15
    and x8, x8, x13
    and x9, x9, x13
    or x8, x8, x14
    or x9, x9, x15
    fmv.d.x f12, x8
    fmv.d.x f13, x9
    and x30, x30, x13
    and x31, x31, x13
    or x30, x30, x14
    or x31, x31, x15
    fmv.d.x f14, x30
    fmv.d.x f15, x31
    ld x28, 144(x3)
    ld x29, 152(x3)
    ld x30, 160(x3)
    ld x31, 168(x3)
rxrv_randomx_riscv64_data_read:
    xor x8, x20, x22
    lwu x1, 92(x3)
    slli x8, x8, 32
    srli x8, x8, 32
rxrv_randomx_riscv64_data_read_v2_tweak:
    slli x8, x8, 32
    xor x25, x25, x8
    ld x8, 0(x7)
    ld x9, 8(x7)
    ld x30, 16(x7)
    ld x31, 24(x7)
    xor x16, x16, x8
    ld x8, 32(x7)
    xor x17, x17, x9
    ld x9, 40(x7)
    xor x18, x18, x30
    ld x30, 48(x7)
    xor x19, x19, x31
    ld x31, 56(x7)
    xor x20, x20, x8
    and x7, x25, x1
    xor x21, x21, x9
    add x7, x7, x6
    xor x22, x22, x30
    xor x23, x23, x31
    srli x9, x25, 32
    slli x25, x25, 32
    or x25, x25, x9
rxrv_randomx_riscv64_data_read_light:
    xor x8, x20, x22
    lui x9, 0x02000
    addi x9, x9, -64
    lwu x1, 92(x3)
    srli x31, x25, 32
    slli x25, x25, 32
    or x25, x25, x31
rxrv_randomx_riscv64_data_read_light_v1:
    slli x8, x8, 32
    xor x25, x25, x8
    and x7, x25, x1
    srli x7, x7, 6
    add x7, x7, x9
rxrv_randomx_riscv64_data_read_light_v2:
    and x7, x25, x1
    srli x7, x7, 6
    add x7, x7, x9
    and x8, x8, x1
    xor x25, x25, x8
rxrv_randomx_riscv64_fix_loop_call:
    jal superscalar_hash
    xor x16, x16, x8
    xor x17, x17, x9
    xor x18, x18, x10
    xor x19, x19, x11
    xor x20, x20, x12
    xor x21, x21, x13
    xor x22, x22, x14
    xor x23, x23, x15
    lwu x10, 80(x3)
    lwu x11, 84(x3)
    ld x12, 96(x3)
    ld x13, 104(x3)
    ld x14, 112(x3)
    ld x15, 120(x3)
rxrv_randomx_riscv64_spad_store:
    sd x16, 0(x27)
    sd x17, 8(x27)
    sd x18, 16(x27)
    sd x19, 24(x27)
    sd x20, 32(x27)
    sd x21, 40(x27)
    sd x22, 48(x27)
    sd x23, 56(x27)
    fmv.x.d x8, f0
    fmv.x.d x9, f8
    fmv.x.d x30, f1
    fmv.x.d x31, f9
    xor x8, x8, x9
    xor x30, x30, x31
    sd x8, 0(x26)
    fmv.d.x f0, x8
    sd x30, 8(x26)
    fmv.d.x f1, x30
    fmv.x.d x8, f2
    fmv.x.d x9, f10
    fmv.x.d x30, f3
    fmv.x.d x31, f11
    xor x8, x8, x9
    xor x30, x30, x31
    sd x8, 16(x26)
    fmv.d.x f2, x8
    sd x30, 24(x26)
    fmv.d.x f3, x30
    fmv.x.d x8, f4
    fmv.x.d x9, f12
    fmv.x.d x30, f5
    fmv.x.d x31, f13
    xor x8, x8, x9
    xor x30, x30, x31
    sd x8, 32(x26)
    fmv.d.x f4, x8
    sd x30, 40(x26)
    fmv.d.x f5, x30
    fmv.x.d x8, f6
    fmv.x.d x9, f14
    fmv.x.d x30, f7
    fmv.x.d x31, f15
    xor x8, x8, x9
    xor x30, x30, x31
    sd x8, 48(x26)
    fmv.d.x f6, x8
    sd x30, 56(x26)
    fmv.d.x f7, x30
rxrv_randomx_riscv64_spad_store_softaes:
    sd x16, 0(x27)
    sd x17, 8(x27)
    sd x18, 16(x27)
    sd x19, 24(x27)
    sd x20, 32(x27)
    sd x21, 40(x27)
    sd x22, 48(x27)
    sd x23, 56(x27)
    fmv.x.d x8, f8
    fmv.x.d x10, f9
    fmv.x.d x30, f0
    fmv.x.d x31, f1
    jal softaes_enc
    fmv.x.d x8, f10
    fmv.x.d x10, f11
    jal softaes_enc
    fmv.x.d x8, f12
    fmv.x.d x10, f13
    jal softaes_enc
    fmv.x.d x8, f14
    fmv.x.d x10, f15
    jal softaes_enc
    sd x30, 0(x26)
    fmv.d.x f0, x30
    sd x31, 8(x26)
    fmv.d.x f1, x31
    fmv.x.d x8, f8
    fmv.x.d x10, f9
    fmv.x.d x30, f2
    fmv.x.d x31, f3
    jal softaes_dec
    fmv.x.d x8, f10
    fmv.x.d x10, f11
    jal softaes_dec
    fmv.x.d x8, f12
    fmv.x.d x10, f13
    jal softaes_dec
    fmv.x.d x8, f14
    fmv.x.d x10, f15
    jal softaes_dec
    sd x30, 16(x26)
    fmv.d.x f2, x30
    sd x31, 24(x26)
    fmv.d.x f3, x31
    fmv.x.d x8, f8
    fmv.x.d x10, f9
    fmv.x.d x30, f4
    fmv.x.d x31, f5
    jal softaes_enc
    fmv.x.d x8, f10
    fmv.x.d x10, f11
    jal softaes_enc
    fmv.x.d x8, f12
    fmv.x.d x10, f13
    jal softaes_enc
    fmv.x.d x8, f14
    fmv.x.d x10, f15
    jal softaes_enc
    sd x30, 32(x26)
    fmv.d.x f4, x30
    sd x31, 40(x26)
    fmv.d.x f5, x31
    fmv.x.d x8, f8
    fmv.x.d x10, f9
    fmv.x.d x30, f6
    fmv.x.d x31, f7
    jal softaes_dec
    fmv.x.d x8, f10
    fmv.x.d x10, f11
    jal softaes_dec
    fmv.x.d x8, f12
    fmv.x.d x10, f13
    jal softaes_dec
    fmv.x.d x8, f14
    fmv.x.d x10, f15
    jal softaes_dec
    sd x30, 48(x26)
    fmv.d.x f6, x30
    sd x31, 56(x26)
    fmv.d.x f7, x31
    lwu x10, 80(x3)
    lwu x11, 84(x3)
    ld x12, 96(x3)
    ld x13, 104(x3)
    ld x14, 112(x3)
    ld x15, 120(x3)
rxrv_randomx_riscv64_loop_end:
    xor x26, x16, x18
    lwu x1, 88(x3)
    addi x24, x24, -1
    srli x27, x26, 32
    and x26, x26, x1
    and x27, x27, x1
    add x26, x26, x5
    add x27, x27, x5
    addi x1, x1, 56
    j condition_check
rxrv_randomx_riscv64_fix_continue_loop:
continue_loop:
    .word 0
condition_check:
    bnez x24, continue_loop
rxrv_randomx_riscv64_epilogue:
    ld x10, 8(sp)
    ld x1, 0(sp)
    ld x3, 16(sp)
    ld x8, 32(sp)
    ld x9, 40(sp)
    ld x24, 96(sp)
    ld x25, 104(sp)
    ld x26, 112(sp)
    ld x27, 120(sp)
    fld f18, 144(sp)
    fld f19, 152(sp)
    fld f20, 160(sp)
    fld f21, 168(sp)
    fld f22, 176(sp)
    fld f23, 184(sp)
    fld f24, 192(sp)
    fld f25, 200(sp)
    fld f26, 208(sp)
    fld f27, 216(sp)
    sd x16, 0(x10)
    sd x17, 8(x10)
    sd x18, 16(x10)
    sd x19, 24(x10)
    sd x20, 32(x10)
    sd x21, 40(x10)
    sd x22, 48(x10)
    sd x23, 56(x10)
    fsd f0, 64(x10)
    fsd f1, 72(x10)
    fsd f2, 80(x10)
    fsd f3, 88(x10)
    fsd f4, 96(x10)
    fsd f5, 104(x10)
    fsd f6, 112(x10)
    fsd f7, 120(x10)
    fsd f8, 128(x10)
    fsd f9, 136(x10)
    fsd f10, 144(x10)
    fsd f11, 152(x10)
    fsd f12, 160(x10)
    fsd f13, 168(x10)
    fsd f14, 176(x10)
    fsd f15, 184(x10)
    ld x18, 48(sp)
    ld x19, 56(sp)
    ld x20, 64(sp)
    ld x21, 72(sp)
    ld x22, 80(sp)
    ld x23, 88(sp)
    fld f8, 128(sp)
    fld f9, 136(sp)
    addi sp, sp, 224
    ret
rxrv_randomx_riscv64_softaes:
softaes_enc:
    ld x13, 128(x3)
    srli x9, x8, 32
    srli x11, x10, 32
    slli x8, x8, 32
    slli x10, x10, 32
    srli x8, x8, 32
    srli x10, x10, 32
    andi x14, x30, 255
    srli x30, x30, 8
    addi x12, x13, -2048
    slli x14, x14, 2
    add x14, x14, x13
    lwu x14, -2048(x14)
    andi x15, x30, 255
    srli x30, x30, 8
    slli x15, x15, 2
    add x15, x15, x12
    lwu x15, 1024(x15)
    xor x8, x8, x14
    andi x14, x30, 255
    srli x30, x30, 8
    slli x14, x14, 2
    add x14, x14, x13
    lwu x14, 0(x14)
    xor x11, x11, x15
    andi x15, x30, 255
    srli x30, x30, 8
    slli x15, x15, 2
    add x15, x15, x13
    lwu x15, 1024(x15)
    xor x10, x10, x14
    andi x14, x30, 255
    srli x30, x30, 8
    slli x14, x14, 2
    add x14, x14, x12
    lwu x14, 0(x14)
    xor x9, x9, x15
    andi x15, x30, 255
    srli x30, x30, 8
    slli x15, x15, 2
    add x15, x15, x12
    lwu x15, 1024(x15)
    xor x9, x9, x14
    andi x14, x30, 255
    srli x30, x30, 8
    slli x14, x14, 2
    add x14, x14, x13
    lwu x14, 0(x14)
    xor x8, x8, x15
    andi x15, x30, 255
    slli x15, x15, 2
    add x15, x15, x13
    lwu x15, 1024(x15)
    xor x11, x11, x14
    andi x14, x31, 255
    srli x31, x31, 8
    slli x14, x14, 2
    add x14, x14, x12
    lwu x14, 0(x14)
    xor x10, x10, x15
    andi x15, x31, 255
    srli x31, x31, 8
    slli x15, x15, 2
    add x15, x15, x12
    lwu x15, 1024(x15)
    xor x10, x10, x14
    andi x14, x31, 255
    srli x31, x31, 8
    slli x14, x14, 2
    add x14, x14, x13
    lwu x14, 0(x14)
    xor x9, x9, x15
    andi x15, x31, 255
    srli x31, x31, 8
    slli x15, x15, 2
    add x15, x15, x13
    lwu x15, 1024(x15)
    xor x8, x8, x14
    andi x14, x31, 255
    srli x31, x31, 8
    slli x14, x14, 2
    add x14, x14, x12
    lwu x14, 0(x14)
    xor x11, x11, x15
    andi x15, x31, 255
    srli x31, x31, 8
    slli x15, x15, 2
    add x15, x15, x12
    lwu x15, 1024(x15)
    xor x11, x11, x14
    andi x14, x31, 255
    srli x31, x31, 8
    slli x14, x14, 2
    add x14, x14, x13
    lwu x14, 0(x14)
    xor x10, x10, x15
    andi x15, x31, 255
    slli x15, x15, 2
    add x15, x15, x13
    lwu x15, 1024(x15)
    xor x9, x9, x14
    slli x11, x11, 32
    slli x9, x9, 32
    or x30, x8, x9
    or x31, x10, x11
    xor x30, x30, x15
    ret
softaes_dec:
    ld x13, 136(x3)
    srli x9, x8, 32
    srli x11, x10, 32
    slli x8, x8, 32
    slli x10, x10, 32
    srli x8, x8, 32
    srli x10, x10, 32
    andi x14, x30, 255
    srli x30, x30, 8
    addi x12, x13, -2048
    slli x14, x14, 2
    add x14, x14, x13
    lwu x14, -2048(x14)
    andi x15, x30, 255
    srli x30, x30, 8
    slli x15, x15, 2
    add x15, x15, x12
    lwu x15, 1024(x15)
    xor x8, x8, x14
    andi x14, x30, 255
    srli x30, x30, 8
    slli x14, x14, 2
    add x14, x14, x13
    lwu x14, 0(x14)
    xor x9, x9, x15
    andi x15, x30, 255
    srli x30, x30, 8
    slli x15, x15, 2
    add x15, x15, x13
    lwu x15, 1024(x15)
    xor x10, x10, x14
    andi x14, x30, 255
    srli x30, x30, 8
    slli x14, x14, 2
    add x14, x14, x12
    lwu x14, 0(x14)
    xor x11, x11, x15
    andi x15, x30, 255
    srli x30, x30, 8
    slli x15, x15, 2
    add x15, x15, x12
    lwu x15, 1024(x15)
    xor x9, x9, x14
    andi x14, x30, 255
    srli x30, x30, 8
    slli x14, x14, 2
    add x14, x14, x13
    lwu x14, 0(x14)
    xor x10, x10, x15
    andi x15, x30, 255
    slli x15, x15, 2
    add x15, x15, x13
    lwu x15, 1024(x15)
    xor x11, x11, x14
    andi x14, x31, 255
    srli x31, x31, 8
    slli x14, x14, 2
    add x14, x14, x12
    lwu x14, 0(x14)
    xor x8, x8, x15
    andi x15, x31, 255
    srli x31, x31, 8
    slli x15, x15, 2
    add x15, x15, x12
    lwu x15, 1024(x15)
    xor x10, x10, x14
    andi x14, x31, 255
    srli x31, x31, 8
    slli x14, x14, 2
    add x14, x14, x13
    lwu x14, 0(x14)
    xor x11, x11, x15
    andi x15, x31, 255
    srli x31, x31, 8
    slli x15, x15, 2
    add x15, x15, x13
    lwu x15, 1024(x15)
    xor x8, x8, x14
    andi x14, x31, 255
    srli x31, x31, 8
    slli x14, x14, 2
    add x14, x14, x12
    lwu x14, 0(x14)
    xor x9, x9, x15
    andi x15, x31, 255
    srli x31, x31, 8
    slli x15, x15, 2
    add x15, x15, x12
    lwu x15, 1024(x15)
    xor x11, x11, x14
    andi x14, x31, 255
    srli x31, x31, 8
    slli x14, x14, 2
    add x14, x14, x13
    lwu x14, 0(x14)
    xor x8, x8, x15
    andi x15, x31, 255
    slli x15, x15, 2
    add x15, x15, x13
    lwu x15, 1024(x15)
    xor x9, x9, x14
    slli x11, x11, 32
    slli x9, x9, 32
    or x30, x8, x9
    or x31, x10, x11
    xor x31, x31, x15
    ret
rxrv_randomx_riscv64_program_end:
    nop
ssh_literal_pool:
    .fill 256,8,0
rxrv_randomx_riscv64_ssh_init:
superscalar_hash:
    ld x30, 0(x3)
    addi x8, x7, 1
    ld x9, 8(x3)
    li x31, (262144*16-1)
    ld x10, 16(x3)
    ld x11, 24(x3)
    mul x8, x8, x30
    ld x12, 32(x3)
    ld x13, 40(x3)
    lla x30, ssh_literal_pool
    ld x14, 48(x3)
    and x7, x7, x31
    ld x15, 56(x3)
    slli x7, x7, 6
    xor x9, x9, x8
    add x7, x7, x6
    xor x10, x10, x8
    ld x31, 2040(x30)
    xor x11, x11, x8
    xor x12, x12, x8
    xor x13, x13, x8
    xor x14, x14, x8
    xor x15, x15, x8
rxrv_randomx_riscv64_ssh_load:
    ld x28, 0(x7)
    ld x29, 8(x7)
    xor x8, x8, x28
    ld x28, 16(x7)
    xor x9, x9, x29
    ld x29, 24(x7)
    xor x10, x10, x28
    ld x28, 32(x7)
    xor x11, x11, x29
    ld x29, 40(x7)
    xor x12, x12, x28
    ld x28, 48(x7)
    xor x13, x13, x29
    ld x29, 56(x7)
    xor x14, x14, x28
    li x7, (262144*16-1)
    xor x15, x15, x29
rxrv_randomx_riscv64_ssh_prefetch:
    and x7, x8, x7
    slli x7, x7, 6
    add x7, x7, x6
rxrv_randomx_riscv64_ssh_end:
    nop
.section .note.GNU-stack,"",%progbits
"#,
    options(raw)
);
unsafe extern "C" {
    static rxrv_randomx_riscv64_literals: u8;
    static rxrv_randomx_riscv64_literals_end: u8;
    static rxrv_randomx_riscv64_data_init: u8;
    static rxrv_randomx_riscv64_fix_data_call: u8;
    static rxrv_randomx_riscv64_prologue: u8;
    static rxrv_randomx_riscv64_loop_begin: u8;
    static rxrv_randomx_riscv64_data_read: u8;
    static rxrv_randomx_riscv64_data_read_v2_tweak: u8;
    static rxrv_randomx_riscv64_data_read_light: u8;
    static rxrv_randomx_riscv64_data_read_light_v1: u8;
    static rxrv_randomx_riscv64_data_read_light_v2: u8;
    static rxrv_randomx_riscv64_fix_loop_call: u8;
    static rxrv_randomx_riscv64_spad_store: u8;
    static rxrv_randomx_riscv64_spad_store_softaes: u8;
    static rxrv_randomx_riscv64_loop_end: u8;
    static rxrv_randomx_riscv64_fix_continue_loop: u8;
    static rxrv_randomx_riscv64_epilogue: u8;
    static rxrv_randomx_riscv64_softaes: u8;
    static rxrv_randomx_riscv64_program_end: u8;
    static rxrv_randomx_riscv64_ssh_init: u8;
    static rxrv_randomx_riscv64_ssh_load: u8;
    static rxrv_randomx_riscv64_ssh_prefetch: u8;
    static rxrv_randomx_riscv64_ssh_end: u8;
}
pub(super) fn literals() -> usize {
    (&raw const rxrv_randomx_riscv64_literals).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn literals_end() -> usize {
    (&raw const rxrv_randomx_riscv64_literals_end).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn data_init() -> usize {
    (&raw const rxrv_randomx_riscv64_data_init).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn fix_data_call() -> usize {
    (&raw const rxrv_randomx_riscv64_fix_data_call).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn prologue() -> usize {
    (&raw const rxrv_randomx_riscv64_prologue).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn loop_begin() -> usize {
    (&raw const rxrv_randomx_riscv64_loop_begin).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn data_read() -> usize {
    (&raw const rxrv_randomx_riscv64_data_read).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn data_read_v2_tweak() -> usize {
    (&raw const rxrv_randomx_riscv64_data_read_v2_tweak).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn data_read_light() -> usize {
    (&raw const rxrv_randomx_riscv64_data_read_light).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn data_read_light_v1() -> usize {
    (&raw const rxrv_randomx_riscv64_data_read_light_v1).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn data_read_light_v2() -> usize {
    (&raw const rxrv_randomx_riscv64_data_read_light_v2).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn fix_loop_call() -> usize {
    (&raw const rxrv_randomx_riscv64_fix_loop_call).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn spad_store() -> usize {
    (&raw const rxrv_randomx_riscv64_spad_store).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn spad_store_softaes() -> usize {
    (&raw const rxrv_randomx_riscv64_spad_store_softaes).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn loop_end() -> usize {
    (&raw const rxrv_randomx_riscv64_loop_end).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn fix_continue_loop() -> usize {
    (&raw const rxrv_randomx_riscv64_fix_continue_loop).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn epilogue() -> usize {
    (&raw const rxrv_randomx_riscv64_epilogue).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn softaes() -> usize {
    (&raw const rxrv_randomx_riscv64_softaes).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn program_end() -> usize {
    (&raw const rxrv_randomx_riscv64_program_end).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn ssh_init() -> usize {
    (&raw const rxrv_randomx_riscv64_ssh_init).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn ssh_load() -> usize {
    (&raw const rxrv_randomx_riscv64_ssh_load).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn ssh_prefetch() -> usize {
    (&raw const rxrv_randomx_riscv64_ssh_prefetch).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn ssh_end() -> usize {
    (&raw const rxrv_randomx_riscv64_ssh_end).addr()
        - (&raw const rxrv_randomx_riscv64_literals).addr()
}
pub(super) fn part(start: usize, end: usize) -> &'static [u8] {
    assert!(start <= end && end <= ssh_end());
    // SAFETY: offsets delimit the single immutable assembler template.
    unsafe {
        std::slice::from_raw_parts(
            (&raw const rxrv_randomx_riscv64_literals).add(start),
            end - start,
        )
    }
}
