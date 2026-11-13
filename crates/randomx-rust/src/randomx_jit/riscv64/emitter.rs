// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2021, SChernykh <https://github.com/SChernykh>
// Copyright (c) 2026 RxMiner contributors
// RV64GC emitter port. See ../../randomx_core/LICENSE.
use super::templates as t;
use crate::randomx_core::{
    cache::Cache,
    superscalar::Operation,
    vm::{self, Kind},
};
pub(super) const PROGRAM_SIZE: usize = 65536;
pub(super) const TOTAL_SIZE: usize = 262144;
const SS: usize = 65536;
const ADD: u32 = 0x33;
const SUB: u32 = 0x40000033;
const XOR: u32 = 0x4033;
const MUL: u32 = 0x2000033;
const HIGH: u32 = 0x2003033;
const SIGNED: u32 = 0x2001033;
fn r(op: u32, d: u32, a: u32, b: u32) -> u32 {
    op | (d << 7) | (a << 15) | (b << 20)
}
fn reg(n: usize) -> u32 {
    if n == 8 {
        8
    } else {
        n as u32 + 16
    }
}
pub(super) struct ProgramConfig {
    pub read: [usize; 4],
    pub masks: [u64; 2],
    pub offset: u32,
    pub v2: bool,
    pub light: bool,
}
pub(super) struct Emitter {
    pub code: Vec<u8>,
    pos: usize,
    pub prefetch_mode: u8,
}
impl Emitter {
    pub fn new(light: bool) -> Self {
        let mut out = Self {
            code: vec![0; if light { TOTAL_SIZE } else { PROGRAM_SIZE }],
            pos: 2048,
            prefetch_mode: 1,
        };
        out.bytes(t::part(t::literals(), t::data_read()));
        out.jump_at(4096 + t::fix_data_call() - t::data_init(), SS, 1);
        out
    }
    pub fn entry() -> usize {
        2048 + t::prologue()
    }
    fn word(&mut self, v: u32) {
        self.code[self.pos..self.pos + 4].copy_from_slice(&v.to_le_bytes());
        self.pos += 4;
    }
    fn bytes(&mut self, b: &[u8]) {
        self.code[self.pos..self.pos + b.len()].copy_from_slice(b);
        self.pos += b.len();
    }
    fn part(&mut self, a: usize, b: usize) {
        self.bytes(t::part(a, b));
    }
    fn at(&mut self, p: usize, v: u32) {
        self.code[p..p + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn op(&mut self, op: u32, d: u32, a: u32, b: u32) {
        self.word(r(op, d, a, b));
    }
    fn immediate(&mut self, op: u32, d: u32, a: u32, n: i32) {
        self.word(r(op, d, a, n as u32 & 4095));
    }
    fn mov(&mut self, d: u32, v: u64) {
        let value = v as i64;
        if i32::try_from(value).is_ok() {
            let low = (value << 52) >> 52;
            let high = ((value - low) >> 12) as u32;
            if high != 0 {
                self.word(0x37 | (d << 7) | (high << 12));
                self.immediate(0x1b, d, d, low as i32);
            } else {
                self.immediate(0x13, d, 0, low as i32);
            }
        } else {
            let low = (value << 52) >> 52;
            let high = ((value as i128 - low as i128) >> 12) as i64;
            self.mov(d, high as u64);
            self.immediate(0x1013, d, d, 12);
            if low != 0 {
                self.immediate(0x13, d, d, low as i32);
            }
        }
    }
    fn jump_at(&mut self, p: usize, target: usize, link: u32) {
        let n = (target as i64 - p as i64) as u32;
        assert!((-1048576..1048576).contains(&(target as i64 - p as i64)));
        self.at(
            p,
            0x6f | (link << 7)
                | ((n & 0x100000) << 11)
                | ((n & 0x7fe) << 20)
                | ((n & 0x800) << 9)
                | (n & 0xff000),
        );
    }
    fn branch_at(&mut self, p: usize, target: usize, a: u32, b: u32, not_equal: bool) {
        let n = (target as i64 - p as i64) as u32;
        assert!((-4096..4096).contains(&(target as i64 - p as i64)));
        self.at(
            p,
            0x63 | if not_equal { 0x1000 } else { 0 }
                | (a << 15)
                | (b << 20)
                | ((n & 0x1000) << 19)
                | ((n & 0x7e0) << 20)
                | ((n & 0x1e) << 7)
                | ((n & 0x800) >> 4),
        );
    }
    fn address(&mut self, src: usize, imm: u64, mask: usize) {
        self.mov(9, imm & (mask as u64 | 7));
        if src != 8 {
            self.op(ADD, 9, 9, reg(src));
        }
        let maskreg = match mask {
            16376 => 10,
            262136 => 11,
            _ => 1,
        };
        self.op(0x7033, 9, 9, maskreg);
        self.op(ADD, 9, 9, 5);
    }
    fn load(&mut self, src: usize, imm: u64, mask: usize) {
        self.address(src, imm, mask);
        self.immediate(0x3003, 8, 9, 0);
    }
    fn float_load(&mut self, src: usize, imm: u64, mask: usize) {
        self.address(src, imm, mask);
        self.immediate(0x2003, 8, 9, 0);
        self.immediate(0x2003, 9, 9, 4);
        self.op(0xd2000053, 24, 8, 0);
        self.op(0xd2000053, 25, 9, 0);
    }
    fn rotate(&mut self, d: u32, s: u32, n: u32, tmp: u32) {
        self.immediate(0x5013, tmp, s, (n & 63) as i32);
        self.immediate(0x1013, d, s, (n.wrapping_neg() & 63) as i32);
        self.op(0x6033, d, d, tmp);
    }
    pub fn compile(&mut self, program: &[u8], config: ProgramConfig) {
        let ProgramConfig {
            read,
            masks,
            offset,
            v2,
            light,
        } = config;
        assert_eq!(program.len(), if v2 { 384 * 8 } else { 256 * 8 });
        self.pos = 2048 + t::literals_end();
        for mask in masks {
            self.bytes(&mask.to_le_bytes());
        }
        self.bytes(&(super::tables::ENCODE[2].as_ptr() as usize).to_le_bytes());
        self.bytes(&(super::tables::DECODE[2].as_ptr() as usize).to_le_bytes());
        self.pos = 2048 + t::data_read();
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
            let d = reg(dst);
            let s = reg(src);
            use Kind::*;
            match kind {
                AddShift => {
                    self.immediate(0x1013, 8, s, shift as i32);
                    self.op(ADD, d, d, 8);
                    if imm != 0 {
                        self.mov(8, imm);
                        self.op(ADD, d, d, 8);
                    }
                }
                AddMem | SubMem | MulMem | HighMem | SignedHighMem | XorMem => {
                    self.load(src, imm, mask);
                    let op = match kind {
                        AddMem => ADD,
                        SubMem => SUB,
                        MulMem => MUL,
                        HighMem => HIGH,
                        SignedHighMem => SIGNED,
                        _ => XOR,
                    };
                    self.op(op, d, d, 8);
                }
                Sub | Mul | High | SignedHigh | Xor => {
                    if src == 8 {
                        self.mov(8, imm);
                    }
                    let op = match kind {
                        Sub => SUB,
                        Mul => MUL,
                        High => HIGH,
                        SignedHigh => SIGNED,
                        _ => XOR,
                    };
                    self.op(op, d, d, s);
                }
                Neg => self.op(SUB, d, 0, d),
                Ror | Rol => {
                    if src == 8 {
                        let n = if kind == Rol { imm.wrapping_neg() } else { imm };
                        self.rotate(d, d, n as u32, 8);
                    } else {
                        self.op(SUB, 8, 0, s);
                        self.op(if kind == Rol { 0x1033 } else { 0x5033 }, 9, d, s);
                        self.op(if kind == Rol { 0x5033 } else { 0x1033 }, d, d, 8);
                        self.op(0x6033, d, d, 9);
                    }
                }
                Swap => {
                    self.immediate(0x13, 8, d, 0);
                    self.immediate(0x13, d, s, 0);
                    self.immediate(0x13, s, 8, 0);
                }
                FloatSwap => {
                    let d = 2 * dst as u32;
                    self.op(0x22000053, 24, d, d);
                    self.op(0x22000053, d, d + 1, d + 1);
                    self.op(0x22000053, d + 1, 24, 24);
                }
                FloatAdd | FloatSub | FloatAddMem | FloatSubMem => {
                    let d = dst as u32 * 2;
                    let s = if matches!(kind, FloatAddMem | FloatSubMem) {
                        self.float_load(src, imm, mask);
                        24
                    } else {
                        16 + 2 * src as u32
                    };
                    let op = if matches!(kind, FloatSub | FloatSubMem) {
                        0x0a007053
                    } else {
                        0x02007053
                    };
                    self.op(op, d, d, s);
                    self.op(op, d + 1, d + 1, s + 1);
                }
                FloatScale => {
                    for d in [dst as u32 * 2, dst as u32 * 2 + 1] {
                        self.op(0xe2000053, 8, d, 0);
                        self.op(XOR, 8, 8, 12);
                        self.op(0xf2000053, d, 8, 0);
                    }
                }
                FloatMul => {
                    for lane in 0..2 {
                        let d = 8 + dst as u32 * 2 + lane;
                        self.op(0x12007053, d, d, 16 + src as u32 * 2 + lane);
                    }
                }
                FloatDiv => {
                    self.float_load(src, imm, mask);
                    for lane in 0..2 {
                        let temp = 8 + lane;
                        let f = 24 + lane;
                        let d = 8 + dst as u32 * 2 + lane;
                        self.op(0xe2000053, temp, f, 0);
                        self.op(0x7033, temp, temp, 13);
                        self.op(0x6033, temp, temp, 14 + lane);
                        self.op(0xf2000053, f, temp, 0);
                        self.op(0x1a007053, d, d, f);
                    }
                }
                FloatSqrt => {
                    for lane in 0..2 {
                        let d = 8 + dst as u32 * 2 + lane;
                        self.op(0x5a007053, d, d, 0);
                    }
                }
                Branch => {
                    self.mov(9, imm);
                    self.op(ADD, d, d, 9);
                    self.mov(8, mask as u64);
                    self.op(0x7033, 8, 8, d);
                    let at = self.pos;
                    self.word(0);
                    self.branch_at(at, at + 8, 8, 0, true);
                    let jump = self.pos;
                    self.word(0);
                    self.jump_at(jump, positions[target], 0);
                }
                Round => {
                    self.rotate(8, s, imm as u32, 9);
                    let skip = if v2 {
                        self.immediate(0x7013, 9, 8, 60);
                        let p = self.pos;
                        self.word(0);
                        Some(p)
                    } else {
                        None
                    };
                    self.immediate(0x7013, 8, 8, 3);
                    self.immediate(0x1013, 8, 8, 2);
                    self.op(ADD, 8, 8, 3);
                    self.immediate(0x2003, 8, 8, 64);
                    self.op(0x00201073, 0, 8, 0);
                    if let Some(p) = skip {
                        self.branch_at(p, self.pos, 9, 0, true);
                    }
                }
                Store => {
                    self.address(dst, imm, mask);
                    self.op(0x3023, 0, 9, s);
                }
                Nop => (),
                Reciprocal => unreachable!(),
            }
        }
        let fix = self.pos;
        if light {
            self.part(t::data_read_light(), t::data_read_light_v1());
            self.at(fix, r(XOR, 8, reg(read[2]), reg(read[3])));
            let value = offset as i32 / 64;
            let low = (value << 20) >> 20;
            let high = (value - low) >> 12;
            self.at(fix + 4, 0x37 | (9 << 7) | ((high as u32) << 12));
            self.at(fix + 8, r(0x13, 9, 9, low as u32 & 4095));
            if v2 {
                self.part(t::data_read_light_v2(), t::fix_loop_call());
            } else {
                self.part(t::data_read_light_v1(), t::data_read_light_v2());
            }
            let fix = self.pos;
            self.part(t::fix_loop_call(), t::spad_store());
            self.jump_at(fix, SS, 1);
        } else {
            self.part(t::data_read(), t::data_read_v2_tweak());
            self.at(fix, r(XOR, 8, reg(read[2]), reg(read[3])));
            let fix = self.pos;
            self.part(t::data_read_v2_tweak(), t::data_read_light());
            self.code[fix..fix + 2]
                .copy_from_slice(&(if v2 { 0x1402u16 } else { 1 }).to_le_bytes());
        }
        if v2 {
            self.part(t::spad_store_softaes(), t::loop_end());
        } else {
            self.part(t::spad_store(), t::spad_store_softaes());
        }
        let fix = self.pos;
        self.part(t::loop_end(), t::epilogue());
        self.at(fix, r(XOR, 26, reg(read[0]), reg(read[1])));
        self.jump_at(
            fix + t::fix_continue_loop() - t::loop_end(),
            2048 + t::loop_begin(),
            0,
        );
        self.part(t::epilogue(), t::softaes());
        if v2 {
            self.part(t::softaes(), t::program_end());
        }
        assert!(self.pos <= PROGRAM_SIZE);
    }
    pub fn superscalar(&mut self, cache: &Cache) {
        self.pos = SS;
        self.part(t::ssh_init(), t::ssh_load());
        for (i, program) in cache.programs.iter().enumerate() {
            for ins in program.instructions() {
                let d = 8 + ins.dst as u32;
                let s = 8 + ins.src as u32;
                match ins.opcode {
                    Operation::Sub => self.op(SUB, d, d, s),
                    Operation::Xor => self.op(XOR, d, d, s),
                    Operation::Mul => self.op(MUL, d, d, s),
                    Operation::MulHigh => self.op(HIGH, d, d, s),
                    Operation::SignedMulHigh => self.op(SIGNED, d, d, s),
                    Operation::AddShift => {
                        let n = ins.shift;
                        self.immediate(0x1013, 28, s, n as i32);
                        self.op(ADD, d, d, 28);
                    }
                    Operation::Rotate => self.rotate(d, d, u32::from(ins.shift), 28),
                    Operation::AddConstant => {
                        let v = ins.signed_immediate();
                        self.mov(28, v);
                        self.op(ADD, d, d, 28);
                    }
                    Operation::XorConstant => {
                        let v = ins.signed_immediate();
                        self.mov(28, v);
                        self.op(XOR, d, d, 28);
                    }
                    Operation::MulReciprocal => {
                        let v = program.multiplier(ins);
                        self.mov(28, v);
                        self.op(MUL, d, d, 28);
                    }
                }
            }
            self.part(t::ssh_load(), t::ssh_prefetch());
            if i < 7 {
                let at = self.pos;
                self.part(t::ssh_prefetch(), t::ssh_end());
                self.at(at, r(0x7033, 7, 8 + program.address_register() as u32, 7));
            }
        }
        self.word(0x00008067);
        assert!(self.pos <= TOTAL_SIZE);
    }
    pub fn dataset_initializer(&mut self, cache: &Cache) {
        self.superscalar(cache);
    }
}
