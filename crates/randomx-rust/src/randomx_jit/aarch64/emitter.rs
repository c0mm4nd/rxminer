// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2019, SChernykh <https://github.com/SChernykh>
// Copyright (c) 2026 RxMiner contributors
// ARM64 emitter port. See ../../randomx_core/LICENSE.
use super::templates as t;
use crate::randomx_core::{
    cache::Cache,
    superscalar::Operation,
    vm::{self, Kind},
};
pub(super) const PROGRAM_SIZE: usize = 32768;
pub(super) const TOTAL_SIZE: usize = 131072;
const REG: [u32; 9] = [4, 5, 6, 7, 12, 13, 14, 15, 20];
const ADD: u32 = 0x8b000000;
const SUB: u32 = 0xcb000000;
const XOR: u32 = 0xca000000;
const MUL: u32 = 0x9b007c00;
const UMULH: u32 = 0x9bc07c00;
const SMULH: u32 = 0x9b407c00;
pub(super) struct ProgramConfig {
    pub read: [usize; 4],
    pub offset: u32,
    pub v2: bool,
    pub light: bool,
    pub hardware: bool,
}
pub(super) struct Emitter {
    pub code: Vec<u8>,
    pos: usize,
    pub prefetch_mode: u8,
}
impl Emitter {
    #[cfg(test)]
    pub fn entry() -> usize {
        0
    }
    pub fn new(light: bool) -> Self {
        let mut code = vec![0; if light { TOTAL_SIZE } else { PROGRAM_SIZE }];
        code[..t::program().len()].copy_from_slice(t::program());
        Self {
            code,
            pos: 0,
            prefetch_mode: 1,
        }
    }
    fn word(&mut self, value: u32) {
        self.code[self.pos..self.pos + 4].copy_from_slice(&value.to_le_bytes());
        self.pos += 4;
    }
    fn bytes(&mut self, data: &[u8]) {
        self.code[self.pos..self.pos + data.len()].copy_from_slice(data);
        self.pos += data.len();
    }
    fn at(&mut self, offset: usize, value: u32) {
        self.code[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn mov(&mut self, dst: u32, value: u64) {
        self.word(0xd2800000 | dst | ((value as u32 & 0xffff) << 5));
        for i in 1..4 {
            let part = (value >> (i * 16)) as u32 & 0xffff;
            if part != 0 {
                self.word(0xf2800000 | dst | (i << 21) | (part << 5));
            }
        }
    }
    fn op(&mut self, opcode: u32, dst: u32, a: u32, b: u32) {
        self.word(opcode | dst | (a << 5) | (b << 16));
    }
    fn add_constant(&mut self, dst: u32, src: u32, value: u64, tmp: u32) {
        if value <= 0xffffff {
            self.word(0x91000000 | dst | (src << 5) | ((value as u32 & 4095) << 10));
            if value > 4095 {
                self.word(0x91400000 | dst | (dst << 5) | ((value as u32 >> 12) << 10));
            }
        } else {
            self.mov(tmp, value);
            self.op(ADD, dst, src, tmp);
        }
    }
    fn branch(&mut self, target: usize) {
        self.word(0x14000000 | (((target as i64 - self.pos as i64) / 4) as u32 & 0x3ffffff));
    }
    fn patch_branch(&mut self, at: usize, target: usize) {
        self.at(
            at,
            0x14000000 | (((target as i64 - at as i64) / 4) as u32 & 0x3ffffff),
        );
    }
    fn address(&mut self, src: usize, imm: u64, mask: usize, tmp: u32) {
        if src == 8 {
            self.mov(tmp, imm & mask as u64);
        } else {
            self.add_constant(tmp, REG[src], imm & (mask as u64 | 7), 19);
            let bits = (mask + 8).trailing_zeros();
            self.word(0x927d0000 | tmp | (tmp << 5) | ((bits - 4) << 10));
        }
    }
    fn load(&mut self, src: usize, imm: u64, mask: usize) {
        self.address(src, imm, mask, 20);
        self.word(0xf8606840 | 20 | (20 << 16));
    }
    fn float_load(&mut self, src: usize, imm: u64, mask: usize) {
        self.address(src, imm, mask, 19);
        self.op(ADD, 19, 2, 19);
        self.word(0x69400000 | 19 | (19 << 5) | (20 << 10));
        self.word(0x4e081c00 | 28 | (19 << 5));
        self.word(0x4e181c00 | 28 | (20 << 5));
        self.word(0x4e61d800 | 28 | (28 << 5));
    }
    pub fn compile(&mut self, program: &[u8], config: ProgramConfig) {
        let ProgramConfig {
            read,
            offset,
            v2,
            light,
            hardware,
        } = config;
        assert_eq!(program.len(), if v2 { 384 * 8 } else { 256 * 8 });
        self.code[..t::program().len()].copy_from_slice(t::program());
        let main = t::program_aarch64_main_loop();
        self.at(main + 4, 0x121a0000 | 16 | (10 << 5) | (14 << 10));
        self.at(main + 8, 0x121a0000 | 17 | (20 << 5) | (14 << 10));
        self.pos = t::program_aarch64_vm_instructions();
        let mut instructions = Vec::with_capacity(384);
        vm::compile(program, &mut instructions);
        let mut positions = [0; 385];
        for (index, instr) in instructions.iter().enumerate() {
            positions[index] = self.pos;
            let vm::Instruction {
                kind,
                dst,
                src,
                imm,
                mask,
                shift,
                target,
            } = *instr;
            let d = REG[dst];
            let s = REG[src];
            use Kind::*;
            match kind {
                AddShift => {
                    self.op(ADD | (shift << 10), d, d, s);
                    if imm != 0 {
                        self.add_constant(d, d, imm, 20);
                    }
                }
                AddMem | SubMem | MulMem | HighMem | SignedHighMem | XorMem => {
                    self.load(src, imm, mask);
                    let op = match kind {
                        AddMem => ADD,
                        SubMem => SUB,
                        MulMem => MUL,
                        HighMem => UMULH,
                        SignedHighMem => SMULH,
                        _ => XOR,
                    };
                    self.op(op, d, d, 20);
                }
                Sub | Mul | High | SignedHigh | Xor => {
                    if src == 8 {
                        self.mov(s, imm);
                    }
                    let op = match kind {
                        Sub => SUB,
                        Mul => MUL,
                        High => UMULH,
                        SignedHigh => SMULH,
                        _ => XOR,
                    };
                    self.op(op, d, d, s);
                }
                Neg => self.op(SUB, d, 31, d),
                Ror | Rol => {
                    if src == 8 {
                        let n = if kind == Rol { imm.wrapping_neg() } else { imm } as u32 & 63;
                        self.word(0x93c00000 | d | (d << 5) | (n << 10) | (d << 16));
                    } else {
                        let s = if kind == Rol {
                            self.op(SUB, 20, 31, s);
                            20
                        } else {
                            s
                        };
                        self.op(0x9ac02c00, d, d, s);
                    }
                }
                Swap => {
                    self.op(0xaa0003e0, 20, 0, d);
                    self.op(0xaa0003e0, d, 0, s);
                    self.op(0xaa0003e0, s, 0, 20);
                }
                FloatSwap => {
                    let d = dst as u32 + 16;
                    self.word(0x6e080400 | 28 | (d << 5) | (1 << 14));
                    self.word(0x6e080400 | d | (d << 5) | (1 << 20));
                    self.word(0x6e080400 | d | (28 << 5));
                }
                FloatAdd | FloatSub | FloatAddMem | FloatSubMem => {
                    let d = dst as u32 + 16;
                    let s = if matches!(kind, FloatAddMem | FloatSubMem) {
                        self.float_load(src, imm, mask);
                        28
                    } else {
                        src as u32 + 24
                    };
                    self.op(
                        if matches!(kind, FloatSub | FloatSubMem) {
                            0x4ee0d400
                        } else {
                            0x4e60d400
                        },
                        d,
                        d,
                        s,
                    );
                }
                FloatScale => {
                    let d = dst as u32 + 16;
                    self.op(0x6e201c00, d, d, 31);
                }
                FloatMul => {
                    let d = dst as u32 + 20;
                    self.op(0x6e60dc00, d, d, src as u32 + 24);
                }
                FloatDiv => {
                    let d = dst as u32 + 20;
                    self.float_load(src, imm, mask);
                    self.op(0x6ee01c00, 28, 30, 29);
                    self.op(0x6e60fc00, d, d, 28);
                }
                FloatSqrt => {
                    let d = dst as u32 + 20;
                    self.word(0x6ee1f800 | d | (d << 5));
                }
                Branch => {
                    self.add_constant(d, d, imm, 20);
                    let shift = mask.trailing_zeros();
                    self.word((0xf2781c1f - ((shift - 8) << 16)) | (d << 5));
                    let target = if target == index + 1 {
                        self.pos + 4
                    } else {
                        positions[target]
                    };
                    let delta = ((target as i64 - self.pos as i64) / 4) as u32 & 0x7ffff;
                    self.word(0x54000000 | (delta << 5));
                }
                Round => {
                    self.word(0x93c00000 | 20 | (s << 5) | ((imm as u32 & 63) << 10) | (s << 16));
                    if v2 {
                        self.word(0xf27e0e9f);
                        self.word(0x54000081);
                    }
                    self.word(0xb3580400 | 8 | (20 << 5));
                    self.word(0xdac00000 | 20 | (8 << 5));
                    self.word(0xd51b4400 | 20);
                }
                Store => {
                    self.address(dst, imm, mask, 20);
                    self.word(0xf8206840 | s | (20 << 16));
                }
                Nop => (),
                Reciprocal => unreachable!(),
            }
        }
        self.op(0x4a000000, 20, REG[read[2]], REG[read[3]]);
        self.branch(if light {
            t::program_aarch64_vm_instructions_end_light()
        } else {
            t::program_aarch64_vm_instructions_end()
        });
        assert!(self.pos <= t::program_aarch64_imul_rcp_literals_end() - 96);
        self.at(
            t::program_aarch64_update_sp_mix1(),
            XOR | 10 | (REG[read[0]] << 5) | (REG[read[1]] << 16),
        );
        if light {
            self.at(
                t::program_aarch64_light_cacheline_align_mask(),
                0x121a0000 | 2 | (2 << 5) | (24 << 10),
            );
            self.pos = t::program_aarch64_light_dataset_offset();
            self.word(0x91000000 | 2 | (2 << 5) | (((offset / 64) & 4095) << 10));
            self.word(0x91400000 | 2 | (2 << 5) | (((offset / 64) >> 12) << 10));
            let source = if v2 {
                t::program_aarch64_vm_instructions_end_light_v2()
            } else {
                t::program_aarch64_vm_instructions_end_light_v1()
            };
            self.pos = t::program_aarch64_vm_instructions_end_light_tweak();
            self.bytes(t::part(source, source + 8));
        } else {
            self.at(
                t::program_aarch64_cacheline_align_mask1(),
                0x121a0000 | 20 | (20 << 5) | (24 << 10),
            );
            self.at(
                t::program_aarch64_cacheline_align_mask2(),
                0x121a0000 | 10 | (10 << 5) | (24 << 10),
            );
            let source = if v2 {
                t::program_aarch64_vm_instructions_end_v2()
            } else {
                t::program_aarch64_vm_instructions_end_v1()
            };
            self.pos = t::program_aarch64_vm_instructions_end();
            self.bytes(t::part(source, source + 16));
        }
        let mix = t::program_aarch64_v2_fe_mix();
        if v2 && hardware {
            self.at(mix, 0x4f00041c);
        } else {
            self.patch_branch(
                mix,
                if v2 {
                    t::program_aarch64_v2_fe_mix_soft_aes()
                } else {
                    t::program_aarch64_v1_fe_mix()
                },
            );
        }
        if v2 && !hardware {
            self.pos = t::program_aarch64_aes_lut_pointers();
            self.bytes(&(super::tables::ENCODE.as_ptr() as usize).to_le_bytes());
            self.bytes(&(super::tables::DECODE.as_ptr() as usize).to_le_bytes());
        }
    }
    pub fn superscalar(&mut self, cache: &Cache) {
        self.pos = t::program().len();
        self.bytes(t::part(
            t::calc_dataset_item_aarch64(),
            t::calc_dataset_item_aarch64_prefetch(),
        ));
        for program in &cache.programs {
            self.word(0x92400000 | 11 | (10 << 5) | (21 << 10));
            self.bytes(t::part(
                t::calc_dataset_item_aarch64_prefetch() + 4,
                t::calc_dataset_item_aarch64_mix(),
            ));
            for instr in program.instructions() {
                let d = instr.dst as u32;
                let s = instr.src as u32;
                match instr.opcode {
                    Operation::Sub => self.op(SUB, d, d, s),
                    Operation::Xor => self.op(XOR, d, d, s),
                    Operation::AddShift => self.op(ADD | (u32::from(instr.shift) << 10), d, d, s),
                    Operation::Mul => self.op(MUL, d, d, s),
                    Operation::MulHigh => self.op(UMULH, d, d, s),
                    Operation::SignedMulHigh => self.op(SMULH, d, d, s),
                    Operation::Rotate => {
                        let n = u32::from(instr.shift);
                        self.word(0x93c00000 | d | (d << 5) | ((n & 63) << 10) | (d << 16))
                    }
                    Operation::AddConstant => self.add_constant(d, d, instr.signed_immediate(), 12),
                    Operation::XorConstant => {
                        let v = instr.signed_immediate();
                        self.mov(12, v);
                        self.op(XOR, d, d, 12);
                    }
                    Operation::MulReciprocal => {
                        let v = program.multiplier(instr);
                        self.mov(12, v);
                        self.op(MUL, d, d, 12);
                    }
                }
            }
            self.bytes(t::part(
                t::calc_dataset_item_aarch64_mix(),
                t::calc_dataset_item_aarch64_store_result(),
            ));
            self.word(0xaa0003e0 | 10 | ((program.address_register() as u32) << 16));
        }
        self.bytes(t::part(
            t::calc_dataset_item_aarch64_store_result(),
            t::calc_dataset_item_aarch64_end(),
        ));
        assert!(self.pos <= TOTAL_SIZE);
    }
    pub fn dataset_initializer(&mut self, cache: &Cache) {
        self.superscalar(cache);
    }
}
