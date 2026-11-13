// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2020, tevador <tevador@gmail.com>
// Copyright (c) 2019-2021, SChernykh <https://github.com/SChernykh>
// Copyright (c) 2019-2021, XMRig <https://github.com/xmrig>
// Copyright (c) 2026 RxMiner contributors
// Rust port of the RandomX x86-64 emitter. See ../randomx_core/LICENSE.
use super::templates as t;
use crate::randomx_core::{
    cache::Cache,
    superscalar::Operation,
    vm::{Kind, FREQUENCIES, KINDS},
};

pub(super) const PROGRAM_SIZE: usize = 16384;
pub(super) const TOTAL_SIZE: usize = 81920;
pub(super) const ROUNDING: usize = PROGRAM_SIZE - 64;
pub(super) const RECIPROCALS: usize = ROUNDING - 128;
const MASK_L1: u32 = 16384 - 8;
const MASK_L2: u32 = 262144 - 8;
const MASK_L3: u32 = 2097152 - 8;
#[inline]
fn reciprocal(divisor: u32) -> Option<u64> {
    if divisor == 0 || divisor.is_power_of_two() {
        return None;
    }
    // floor(2^(63 + bit_length(divisor)) / divisor). DIV accepts a 128-bit
    // dividend in RDX:RAX, avoiding the portable implementation's two divisions.
    let high = 1u64 << (31 - divisor.leading_zeros());
    let quotient: u64;
    // SAFETY: divisor is nonzero and high < divisor (powers of two excluded),
    // so the quotient fits u64 and DIV cannot raise a divide exception.
    unsafe {
        std::arch::asm!(
            "div {divisor}",
            divisor = in(reg) u64::from(divisor),
            inout("rax") 0u64 => quotient,
            inout("rdx") high => _,
            options(pure, nomem, nostack),
        );
    }
    Some(quotient)
}
const TYPES: [Kind; 256] = {
    let mut out = [Kind::Nop; 256];
    let mut i = 0;
    let mut kind = 0;
    while kind < KINDS.len() {
        let mut n = 0;
        while n < FREQUENCIES[kind] {
            out[i] = KINDS[kind];
            i += 1;
            n += 1;
        }
        kind += 1;
    }
    out
};

pub(super) struct ProgramConfig {
    pub read: [usize; 4],
    pub masks: [u64; 2],
    pub offset: u32,
    pub v2: bool,
    pub light: bool,
    pub hardware: bool,
}

pub(super) struct Emitter {
    constants_ready: bool,
    pub code: Vec<u8>,
    pub prefetch_mode: u8,
    bmi2: bool,
    zen2: bool,
}
// The cursor lives in this stack-local writer rather than the owning VM.
// Its borrowed output slice cannot alias the cursor, allowing register allocation
// across byte/immediate stores while retaining checked Rust slice accesses.
struct Writer<'a> {
    code: &'a mut [u8],
    pos: usize,
    prefetch_mode: u8,
}
impl Writer<'_> {
    #[inline(always)]
    fn byte(&mut self, v: u8) {
        self.code[self.pos] = v;
        self.pos += 1;
    }
    #[inline(always)]
    fn bytes(&mut self, v: &[u8]) {
        self.code[self.pos..self.pos + v.len()].copy_from_slice(v);
        self.pos += v.len();
    }
    #[inline(always)]
    fn word(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    #[inline(always)]
    fn quad(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    #[inline(always)]
    fn relative(&mut self, target: usize) {
        self.word((target as i32 - self.pos as i32 - 4) as u32);
    }
    #[inline(always)]
    fn sib(&mut self, shift: u8, src: u8, dst: u8) {
        self.byte((shift << 6) | (src << 3) | dst);
    }
    #[inline(always)]
    fn address(&mut self, reg: u8, imm: u32, mask: u32, rax: bool) {
        self.bytes(&[0x41, 0x8d]);
        self.byte(0x80 + reg + if rax { 0 } else { 8 });
        if reg == 4 {
            self.byte(0x24);
        }
        self.word(imm);
        if rax {
            self.byte(0x25);
        } else {
            self.bytes(&[0x81, 0xe1]);
        }
        self.word(mask);
    }
    #[inline(always)]
    fn memory(&mut self, opcode: &[u8], dst: u8, src: u8, imm: u32, mask: u32) {
        if src != dst {
            self.address(src, imm, mask, true);
            self.bytes(opcode);
            self.bytes(&[0x04 + 8 * dst, 0x06]);
        } else {
            self.bytes(opcode);
            self.byte(0x86 + 8 * dst);
            self.word(imm & MASK_L3);
        }
    }
    fn cache_template(&mut self, bytes: &[u8], hint_offset: usize, temporal: bool) {
        let hint = self.pos + hint_offset;
        self.bytes(bytes);
        assert_eq!(&self.code[hint..hint + 3], &[0x0f, 0x18, 0x03]);
        // The caller selects the cache policy independently of scratchpad
        // prefetching. Both instructions preserve the addressing operand.
        if temporal {
            self.code[hint + 2] = 0x0b; // PREFETCHT0 [RBX]
        }
    }
    #[inline(always)]
    fn prefetch(&mut self, sib: u8) {
        match self.prefetch_mode {
            0 => (),
            1 => self.bytes(&[0x0f, 0x18, 0x0c, sib]),
            2 => self.bytes(&[0x0f, 0x18, 0x04, sib]),
            3 => self.bytes(&[0x48, 0x8b, 0x0c, sib]),
            _ => unreachable!("validated prefetch mode"),
        }
    }
}

impl Emitter {
    /// The prefix before these per-program exponent masks never changes.
    pub fn masks_offset() -> usize {
        t::prologue().len() - 48
    }

    #[allow(unused_unsafe)] // __cpuid is unsafe on the Rust 1.88 MSRV.
    pub fn new(light: bool) -> Self {
        // SAFETY: CPUID leaf 1 is present on every x86-64 processor.
        let cpu = unsafe { std::arch::x86_64::__cpuid(1) };
        // SAFETY: CPUID leaf 0 is present on every x86-64 processor.
        let vendor = unsafe { std::arch::x86_64::__cpuid(0) };
        let amd = vendor.ebx == 0x68747541 && vendor.edx == 0x69746e65 && vendor.ecx == 0x444d4163;
        let family = ((cpu.eax >> 8) & 15) + ((cpu.eax >> 20) & 255);
        let model = ((cpu.eax >> 4) & 15) | ((cpu.eax >> 12) & 240);
        Self {
            constants_ready: false,
            code: vec![0; if light { TOTAL_SIZE } else { PROGRAM_SIZE }],
            prefetch_mode: 1,
            bmi2: std::is_x86_feature_detected!("bmi2"),
            zen2: amd && family == 0x17 && (0x30..=0x3f).contains(&model),
        }
    }
    pub fn compile(&mut self, program: &[u8], config: ProgramConfig) -> usize {
        let ProgramConfig {
            read,
            masks,
            offset,
            v2,
            light,
            hardware,
        } = config;
        assert_eq!(program.len(), if v2 { 384 * 8 } else { 256 * 8 });
        assert!(self.code.len() >= PROGRAM_SIZE);
        // The instruction stream, masks and reciprocal slots change per
        // program; the prologue, epilogue and rounding controls do not.
        if !self.constants_ready {
            self.code[..t::prologue().len()].copy_from_slice(t::prologue());
            let epilogue = RECIPROCALS - t::epilogue().len();
            self.code[epilogue..RECIPROCALS].copy_from_slice(t::epilogue());
            for (i, value) in [0x9fc0u32, 0xbfc0, 0xdfc0, 0xffc0].iter().enumerate() {
                self.code[ROUNDING + 4 * i..ROUNDING + 4 * i + 4]
                    .copy_from_slice(&value.to_le_bytes());
            }
            self.constants_ready = true;
        }
        let mut out = Writer {
            code: &mut self.code,
            pos: t::prologue().len(),
            prefetch_mode: self.prefetch_mode,
        };
        let at = out.pos - 48;
        out.code[at..at + 8].copy_from_slice(&masks[0].to_le_bytes());
        out.code[at + 8..at + 16].copy_from_slice(&masks[1].to_le_bytes());
        let stack_reciprocals = !light && (!v2 || hardware);
        if stack_reciprocals {
            out.bytes(t::stack_reciprocals());
        }
        let loop_start = out.pos;
        out.bytes(t::load());
        // Track the code position after each register's last write directly.
        // This replaces the 384-entry instruction-position table and its clears.
        let mut usage = [out.pos; 8];
        let mut reciprocals = 0;
        for (index, raw) in program.as_chunks::<8>().0.iter().enumerate() {
            // Every instruction emits at most 33 bytes. Bound the cursor by
            // the fixed VM region, even while the light helper buffer is live.
            // Unlike len - pos >= 40, this also proves pos cannot exceed len
            // through unsigned wraparound, helping LLVM remove writer checks.
            assert!(out.pos <= PROGRAM_SIZE - 40);
            let kind = TYPES[raw[0] as usize];
            let dst = raw[1] & 7;
            let src = raw[2] & 7;
            let mode = raw[3];
            let imm = u32::from_le_bytes(raw[4..].try_into().unwrap());
            let mask = if mode & 3 != 0 { MASK_L1 } else { MASK_L2 };
            use Kind::*;
            match kind {
                AddShift => {
                    out.bytes(&[0x4f, 0x8d]);
                    out.byte(if dst == 5 { 0xac } else { 0x04 + 8 * dst });
                    out.sib((mode >> 2) & 3, src, dst);
                    if dst == 5 {
                        out.word(imm);
                    }
                    usage[dst as usize] = out.pos;
                }
                AddMem => {
                    out.memory(&[0x4c, 0x03], dst, src, imm, mask);
                    usage[dst as usize] = out.pos;
                }
                Sub | Xor => {
                    if src != dst {
                        out.bytes(&[0x4d, if kind == Sub { 0x2b } else { 0x33 }]);
                        out.byte(0xc0 + 8 * dst + src);
                    } else {
                        out.bytes(&[0x49, 0x81]);
                        out.byte(if kind == Sub { 0xe8 + dst } else { 0xf0 + dst });
                        out.word(imm);
                    }
                    usage[dst as usize] = out.pos;
                }
                SubMem => {
                    out.memory(&[0x4c, 0x2b], dst, src, imm, mask);
                    usage[dst as usize] = out.pos;
                }
                Mul => {
                    if src != dst {
                        out.bytes(&[0x4d, 0x0f, 0xaf]);
                        out.byte(0xc0 + 8 * dst + src);
                    } else {
                        out.bytes(&[0x4d, 0x69]);
                        out.byte(0xc0 + 9 * dst);
                        out.word(imm);
                    }
                    usage[dst as usize] = out.pos;
                }
                MulMem => {
                    out.memory(&[0x4c, 0x0f, 0xaf], dst, src, imm, mask);
                    usage[dst as usize] = out.pos;
                }
                High | SignedHigh => {
                    out.bytes(&[0x49, 0x8b]);
                    out.byte(if self.bmi2 && kind == High {
                        0xd0 + dst
                    } else {
                        0xc0 + dst
                    });
                    if self.bmi2 && kind == High {
                        out.bytes(&[0xc4, 0x42, 0xfb, 0xf6]);
                        out.byte(0xc0 + 8 * dst + src);
                    } else {
                        out.bytes(&[0x49, 0xf7]);
                        out.byte(if kind == High { 0xe0 + src } else { 0xe8 + src });
                        out.bytes(&[0x4c, 0x8b]);
                        out.byte(0xc2 + 8 * dst);
                    }
                    usage[dst as usize] = out.pos;
                }
                HighMem | SignedHighMem => {
                    if src != dst {
                        out.address(src, imm, mask, false);
                    }
                    out.bytes(&[0x49, 0x8b]);
                    out.byte(if self.bmi2 && kind == HighMem {
                        0xd0 + dst
                    } else {
                        0xc0 + dst
                    });
                    if self.bmi2 && kind == HighMem {
                        out.bytes(&[0xc4, 0x62, 0xfb, 0xf6]);
                        if src != dst {
                            out.bytes(&[0x04 + 8 * dst, 0x0e]);
                        } else {
                            out.byte(0x86 + 8 * dst);
                            out.word(imm & MASK_L3);
                        }
                    } else {
                        out.bytes(&[0x48, 0xf7]);
                        if src != dst {
                            out.bytes(&[if kind == HighMem { 0x24 } else { 0x2c }, 0x0e]);
                        } else {
                            out.byte(if kind == HighMem { 0xa6 } else { 0xae });
                            out.word(imm & MASK_L3);
                        }
                        out.bytes(&[0x4c, 0x8b]);
                        out.byte(0xc2 + 8 * dst);
                    }
                    usage[dst as usize] = out.pos;
                }
                Reciprocal => {
                    if let Some(value) = reciprocal(imm) {
                        if reciprocals < 16 {
                            let at = RECIPROCALS + reciprocals * 8;
                            out.code[at..at + 8].copy_from_slice(&value.to_le_bytes());
                            out.bytes(&[0x4c, 0x0f, 0xaf]);
                            if stack_reciprocals {
                                out.bytes(&[
                                    0x44 + 8 * dst,
                                    0x24,
                                    (-8 * (reciprocals as i32 + 1)) as u8,
                                ]);
                            } else {
                                out.byte(0x05 + 8 * dst);
                                out.relative(at);
                            }
                            reciprocals += 1;
                        } else {
                            out.bytes(&[0x48, 0xb8]);
                            out.quad(value);
                            out.bytes(&[0x4c, 0x0f, 0xaf]);
                            out.byte(0xc0 + 8 * dst);
                        }
                        usage[dst as usize] = out.pos;
                    }
                }
                Neg => {
                    out.bytes(&[0x49, 0xf7]);
                    out.byte(0xd8 + dst);
                    usage[dst as usize] = out.pos;
                }
                XorMem => {
                    out.memory(&[0x4c, 0x33], dst, src, imm, mask);
                    usage[dst as usize] = out.pos;
                }
                Ror | Rol => {
                    let op = if kind == Ror { 0xc8 } else { 0xc0 };
                    if src != dst {
                        out.bytes(&[0x41, 0x8b, 0xc8 + src, 0x49, 0xd3, op + dst]);
                    } else {
                        out.bytes(&[0x49, 0xc1, op + dst, (imm & 63) as u8]);
                    }
                    usage[dst as usize] = out.pos;
                }
                Swap => {
                    if src != dst {
                        if self.zen2 && !light && hardware && self.prefetch_mode != 3 {
                            // RAX is temporary between VM instructions. The
                            // measured Zen 2 full/hardware-AES path favors these
                            // copies except with load-prefetch (p3). Both forms
                            // preserve flags and all other VM registers.
                            out.bytes(&[0x4c, 0x89, 0xc0 + 8 * dst]); // MOV RAX,dst.
                            out.bytes(&[0x4d, 0x89, 0xc0 + 8 * src + dst]); // MOV dst,src.
                            out.bytes(&[0x49, 0x89, 0xc0 + src]); // MOV src,RAX.
                        } else {
                            out.bytes(&[0x4d, 0x87, 0xc0 + src + 8 * dst]);
                        }
                        usage[dst as usize] = out.pos;
                        usage[src as usize] = out.pos;
                    }
                }
                FloatSwap => out.bytes(&[0x66, 0x0f, 0xc6, 0xc0 + 9 * dst, 1]),
                FloatAdd | FloatSub | FloatMul => {
                    out.bytes(&[
                        0x66,
                        0x41,
                        0x0f,
                        match kind {
                            FloatAdd => 0x58,
                            FloatSub => 0x5c,
                            _ => 0x59,
                        },
                    ]);
                    out.byte(
                        (if kind == FloatMul { 0xe0 } else { 0xc0 }) + (src & 3) + 8 * (dst & 3),
                    );
                }
                FloatAddMem | FloatSubMem | FloatDiv => {
                    out.address(src, imm, mask, true);
                    out.bytes(&[0xf3, 0x44, 0x0f, 0xe6, 0x24, 0x06]);
                    if kind == FloatDiv {
                        out.bytes(&[0x45, 0x0f, 0x54, 0xe5, 0x45, 0x0f, 0x56, 0xe6]);
                    }
                    out.bytes(&[
                        0x66,
                        0x41,
                        0x0f,
                        match kind {
                            FloatAddMem => 0x58,
                            FloatSubMem => 0x5c,
                            _ => 0x5e,
                        },
                    ]);
                    out.byte((if kind == FloatDiv { 0xe4 } else { 0xc4 }) + 8 * (dst & 3));
                }
                FloatScale => out.bytes(&[0x41, 0x0f, 0x57, 0xc7 + 8 * (dst & 3)]),
                FloatSqrt => out.bytes(&[0x66, 0x0f, 0x51, 0xe4 + 9 * (dst & 3)]),
                Branch => {
                    let target = usage[dst as usize];
                    let shift = (mode >> 4) + 8;
                    let imm = (imm | (1u32 << shift)) & !(1u32 << (shift - 1));
                    out.bytes(&[0x49, 0x81, 0xc0 + dst]);
                    out.word(imm);
                    out.bytes(&[0x49, 0xf7, 0xc0 + dst]);
                    out.word(255 << shift);
                    let delta = target as i32 - out.pos as i32 - 2;
                    if (-128..=127).contains(&delta) {
                        out.bytes(&[0x74, delta as u8]);
                    } else {
                        out.bytes(&[0x0f, 0x84]);
                        out.relative(target);
                    }
                    usage.fill(out.pos);
                }
                Round => {
                    if !v2 {
                        let mut dead = false;
                        for next in program[index * 8 + 8..].as_chunks::<8>().0 {
                            match TYPES[next[0] as usize] {
                                Round => {
                                    dead = true;
                                    break;
                                }
                                FloatAdd | FloatAddMem | FloatSub | FloatSubMem | FloatMul
                                | FloatDiv | FloatSqrt | Branch => break,
                                _ => {}
                            }
                        }
                        if dead {
                            continue;
                        }
                    }
                    if self.zen2 && !v2 {
                        let rotate = (2u32.wrapping_sub(imm & 63)) & 63;
                        if self.bmi2 {
                            out.bytes(&[
                                0xc4,
                                0xc3,
                                0xfb,
                                0xf0,
                                0xc0 + src,
                                ((64 - rotate) & 63) as u8,
                            ]);
                        } else {
                            out.bytes(&[0x49, 0x8b, 0xc0 + src]);
                            if rotate != 0 {
                                out.bytes(&[0x48, 0xc1, 0xc0, rotate as u8]);
                            }
                        }
                        out.bytes(&[0x83, 0xe0, 0x0c, 0x3b, 0x44, 0x24, 0x04, 0x74]);
                        let skip = out.pos;
                        out.byte(0);
                        if !light {
                            out.bytes(&[0x0f, 0xae, 0x54, 0x04, 0x20]);
                        } else {
                            out.bytes(&[0x48, 0x8d, 0x0d]);
                            out.relative(ROUNDING);
                            out.bytes(&[0x0f, 0xae, 0x14, 0x01]);
                        }
                        out.bytes(&[0xeb, 0, 0x89, 0x44, 0x24, 0x04]);
                        out.code[skip] = (out.pos - skip - 1) as u8;
                    } else {
                        out.bytes(&[0x49, 0x8b, 0xc0 + src]);
                        let rotate = (13u32.wrapping_sub(imm & 63)) & 63;
                        if rotate != 0 {
                            out.bytes(&[0x48, 0xc1, 0xc0, rotate as u8]);
                        }
                        if v2 {
                            out.bytes(&[0xa9, 0, 0x80, 7, 0, 0x75, 17]);
                        }
                        out.bytes(&[
                            0x25, 0, 0x60, 0, 0, 0x0d, 0xc0, 0x9f, 0, 0, 0x89, 4, 0x24, 0x0f, 0xae,
                            0x14, 0x24,
                        ]);
                    }
                }
                Store => {
                    out.address(dst, imm, if mode >> 4 >= 14 { MASK_L3 } else { mask }, true);
                    out.bytes(&[0x4c, 0x89, 0x04 + 8 * src, 0x06]);
                }
                Nop => out.byte(0x90),
            }
        }
        out.bytes(&[
            0x41,
            0x8b,
            0xc0 + read[2] as u8,
            0x41,
            0x33,
            0xc0 + read[3] as u8,
        ]);
        if light {
            out.bytes(if v2 { t::light_v2() } else { t::light_v1() });
            out.bytes(&[0x81, 0xc3]);
            out.word(offset / 64);
            out.byte(0xe8);
            out.relative(PROGRAM_SIZE);
            out.bytes(t::light_finish());
        } else {
            out.bytes(if v2 { t::dataset_v2() } else { t::dataset_v1() });
        }
        out.bytes(&[
            0x49,
            0x8b,
            0xc0 + read[0] as u8,
            0x49,
            0x33,
            0xc0 + read[1] as u8,
        ]);
        if self.bmi2 {
            out.bytes(&[0xc4, 0xe3, 0xfb, 0xf0, 0xd0, 0x20]);
        } else {
            out.bytes(&[0x48, 0x89, 0xc2, 0x48, 0xc1, 0xea, 0x20]);
        }
        out.byte(0x25);
        out.word(2097152 - 64);
        out.prefetch(0x06);
        out.bytes(&[0x81, 0xe2]);
        out.word(2097152 - 64);
        out.prefetch(0x16);
        out.bytes(if v2 {
            if hardware {
                t::store_v2()
            } else {
                t::store_soft_v2()
            }
        } else {
            t::store_v1()
        });
        out.bytes(&[0x83, 0xeb, 1, 0x0f, 0x85]);
        out.relative(loop_start);
        out.byte(0xe9);
        let epilogue = RECIPROCALS - t::epilogue().len();
        out.relative(epilogue);
        if v2 && !hardware {
            out.bytes(t::soft_aes());
            out.quad(super::aes::ENCODE.as_ptr() as usize as u64);
            out.quad(super::aes::DECODE.as_ptr() as usize as u64);
        }
        assert!(out.pos <= epilogue, "JIT program overflow");
        out.pos
    }
    /// Light helpers live in the executable mapping after publication. VM
    /// compilation only needs the first 16 KiB of this staging allocation.
    pub fn discard_superscalar_copy(&mut self) {
        self.code.truncate(PROGRAM_SIZE);
        self.code.shrink_to_fit();
    }
    /// Returns the end of the generated helper, including its final RET.
    pub fn superscalar(&mut self, cache: &Cache) -> usize {
        // Light VM cache reads benefit from T0 on the measured Zen 2 path.
        // Keep the existing policy on other CPUs.
        self.superscalar_prefetch(cache, self.zen2, PROGRAM_SIZE)
    }
    fn superscalar_prefetch(&mut self, cache: &Cache, temporal: bool, start: usize) -> usize {
        assert_eq!(self.code.len(), TOTAL_SIZE);
        assert!(start <= PROGRAM_SIZE);
        let mut out = Writer {
            code: &mut self.code,
            pos: start,
            prefetch_mode: self.prefetch_mode,
        };
        out.cache_template(t::ss_init(), t::ss_init_hint_offset(), temporal);
        for (j, program) in cache.programs.iter().enumerate() {
            for instr in program.instructions() {
                let dst = instr.dst;
                let src = instr.src;
                use Operation::*;
                match instr.opcode {
                    Sub | Xor => out.bytes(&[
                        0x4d,
                        if matches!(instr.opcode, Sub) {
                            0x2b
                        } else {
                            0x33
                        },
                        0xc0 + 8 * dst + src,
                    ]),
                    AddShift => {
                        let n = instr.shift;
                        out.bytes(&[0x4f, 0x8d, 0x04 + 8 * dst]);
                        out.sib(n, src, dst);
                    }
                    Mul => out.bytes(&[0x4d, 0x0f, 0xaf, 0xc0 + 8 * dst + src]),
                    Rotate => out.bytes(&[0x49, 0xc1, 0xc8 + dst, instr.shift]),
                    AddConstant | XorConstant => {
                        let n = instr.immediate;
                        out.bytes(&[
                            0x49,
                            0x81,
                            if matches!(instr.opcode, AddConstant) {
                                0xc0 + dst
                            } else {
                                0xf0 + dst
                            },
                        ]);
                        out.word(n);
                    }
                    MulHigh | SignedMulHigh => {
                        out.bytes(&[
                            0x49,
                            0x8b,
                            0xc0 + dst,
                            0x49,
                            0xf7,
                            if matches!(instr.opcode, MulHigh) {
                                0xe0 + src
                            } else {
                                0xe8 + src
                            },
                            0x4c,
                            0x8b,
                            0xc2 + 8 * dst,
                        ]);
                    }
                    MulReciprocal => {
                        let n = program.multiplier(instr);
                        out.bytes(&[0x48, 0xb8]);
                        out.quad(n);
                        out.bytes(&[0x4c, 0x0f, 0xaf, 0xc0 + 8 * dst]);
                    }
                }
            }
            out.bytes(t::ss_load());
            if j != 7 {
                out.bytes(&[0x49, 0x8b, 0xd8 + program.address_register() as u8]);
                out.cache_template(t::ss_prefetch(), t::ss_prefetch_hint_offset(), temporal);
            }
        }
        out.byte(0xc3);
        assert!(out.pos <= TOTAL_SIZE, "JIT superscalar overflow");
        out.pos
    }
    pub fn dataset_initializer(&mut self, cache: &Cache) -> usize {
        // A standalone initializer has no VM program. Put its helper directly
        // after the wrapper, retaining cache-line alignment without a 16 KiB gap.
        let start = t::dataset_init().len().next_multiple_of(64);
        // Parallel bulk initialization retains NTA, including on Zen 2.
        self.dataset_initializer_at(cache, start, false)
    }
    fn dataset_initializer_at(&mut self, cache: &Cache, start: usize, temporal: bool) -> usize {
        assert!(start >= t::dataset_init().len());
        // Initializer code replaces the VM prefix; restore it if reused for a VM.
        self.constants_ready = false;
        let end = self.superscalar_prefetch(cache, temporal, start);
        let mut out = Writer {
            code: &mut self.code,
            pos: 0,
            prefetch_mode: self.prefetch_mode,
        };
        out.bytes(t::dataset_init());
        let call_end = t::dataset_init_call_end();
        assert_eq!(out.code[call_end - 5], 0xe8);
        assert!(call_end <= out.pos);
        out.code[call_end - 4..call_end]
            .copy_from_slice(&((start as i32) - (call_end as i32)).to_le_bytes());
        end
    }
}

#[cfg(test)]
impl Emitter {
    pub(super) fn baseline_instructions(&mut self) {
        self.bmi2 = false;
        self.zen2 = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn four_lane_software_mix_matches_portable_and_preserves_keys() {
        use super::super::memory::Executable;
        use crate::randomx_core::aes::{Aes, Backend};

        // A System V wrapper saves the callee-saved registers borrowed by the
        // helper. Three pushes also align RSP before its four nested calls.
        let mut bytes = vec![0x53, 0x55, 0x41, 0x54, 0x49, 0x89, 0xfc];
        for lane in 0..12u8 {
            // MOVDQU XMMlane, [R12 + displacement]: F, E and invariant A.
            bytes.extend_from_slice(&[
                0xf3,
                if lane < 8 { 0x41 } else { 0x45 },
                0x0f,
                0x6f,
                0x84 + (lane & 7) * 8,
                0x24,
            ]);
            bytes.extend_from_slice(&(u32::from(lane) * 16).to_le_bytes());
        }
        bytes.extend_from_slice(&[0x48, 0xbf]);
        bytes.extend_from_slice(&(super::super::aes::ENCODE.as_ptr() as u64).to_le_bytes());
        bytes.extend_from_slice(&[0x48, 0xbe]);
        bytes.extend_from_slice(&(super::super::aes::DECODE.as_ptr() as u64).to_le_bytes());
        let mut calls = Vec::new();
        for key in 4..8 {
            bytes.extend_from_slice(&[0x66, 0x44, 0x0f, 0x28, 0xe0 + key]);
            calls.push(bytes.len());
            bytes.extend_from_slice(&[0xe8, 0, 0, 0, 0]);
        }
        for lane in 0..12u8 {
            bytes.extend_from_slice(&[
                0xf3,
                if lane < 8 { 0x41 } else { 0x45 },
                0x0f,
                0x7f,
                0x84 + (lane & 7) * 8,
                0x24,
            ]);
            bytes.extend_from_slice(&(u32::from(lane) * 16).to_le_bytes());
        }
        bytes.extend_from_slice(&[0x41, 0x5c, 0x5d, 0x5b, 0xc3]);
        let helper = bytes.len();
        bytes.extend_from_slice(t::soft_aes());
        for call in calls {
            let relative = helper as i32 - (call as i32 + 5);
            bytes[call + 1..call + 5].copy_from_slice(&relative.to_le_bytes());
        }
        let mut code = Executable::new(bytes.len().next_multiple_of(4096)).unwrap();
        code.publish(&bytes).unwrap();
        // SAFETY: the bounded wrapper implements System V and calls only our
        // SSE2 helper. Code owns the live RX mapping for every invocation.
        let run: unsafe extern "sysv64" fn(*mut u8) = unsafe { std::mem::transmute(code.entry()) };
        let aes = Aes::default();
        let mut random = 0x8a317b2051e9c4d6u64;
        for case in 0..512 {
            let mut actual = [0xa7; 256];
            for byte in &mut actual[32..224] {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                *byte = match case {
                    0 => 0,
                    1 => 255,
                    _ => random as u8,
                };
            }
            let mut expected = actual;
            let mut f = std::array::from_fn(|i| {
                u64::from_le_bytes(actual[32 + i * 8..40 + i * 8].try_into().unwrap())
            });
            let e = std::array::from_fn(|i| {
                u64::from_le_bytes(actual[96 + i * 8..104 + i * 8].try_into().unwrap())
            });
            aes.mix(&mut f, &e);
            for (i, word) in f.into_iter().enumerate() {
                expected[32 + i * 8..40 + i * 8].copy_from_slice(&word.to_le_bytes());
            }
            // SAFETY: the wrapper reads/writes exactly 192 initialized bytes;
            // surrounding guards are live and complete unaligned lanes are valid.
            unsafe { run(actual[32..224].as_mut_ptr()) };
            assert_eq!(
                actual, expected,
                "F result, preserved E/A and guards, case {case}"
            );
        }
    }

    #[test]
    #[ignore = "two 256 MiB caches and native relocated helper execution"]
    fn compact_initializer_matches_original_layout() {
        use super::super::{memory::Executable, Initializer};
        use crate::randomx_core::cache::DATASET_ITEMS;
        for key in [b"test key 000".as_slice(), b"compact initializer layout"] {
            let cache = super::super::new_cache(key, None).unwrap();
            let mut original = Emitter::new(true);
            let old_end = original.dataset_initializer_at(&cache, PROGRAM_SIZE, false);
            assert_eq!(&original.code[..t::dataset_init().len()], t::dataset_init());
            let mut compact = Emitter::new(true);
            compact.code.fill(0xcc);
            let end = compact.dataset_initializer(&cache);
            let start = t::dataset_init().len().next_multiple_of(64);
            assert_eq!(start % 64, 0);
            assert_eq!(
                &original.code[PROGRAM_SIZE..old_end],
                &compact.code[start..end]
            );
            let call_end = t::dataset_init_call_end();
            let delta =
                i32::from_le_bytes(compact.code[call_end - 4..call_end].try_into().unwrap());
            assert_eq!(call_end as i32 + delta, start as i32);
            assert_eq!(
                &original.code[..call_end - 4],
                &compact.code[..call_end - 4]
            );
            assert_eq!(
                &original.code[call_end..t::dataset_init().len()],
                &compact.code[call_end..t::dataset_init().len()]
            );
            assert_eq!(compact.code[end - 1], 0xc3);
            assert!(compact.code[end..].iter().all(|&byte| byte == 0xcc));
            assert!(end.next_multiple_of(4096) < old_end.next_multiple_of(4096));
            for secure in [false, true] {
                let mut code =
                    Executable::with_options(end.next_multiple_of(4096), false, secure).unwrap();
                code.publish(&compact.code[..end]).unwrap();
                let init = Initializer {
                    cache: &cache,
                    code,
                };
                for start in [0, 12345, DATASET_ITEMS - 257] {
                    let guard = [0xa5a5_a5a5_a5a5_a5a5; 8];
                    let mut output = [guard; 259];
                    init.fill(start, &mut output[1..258]).unwrap();
                    assert_eq!(output[0], guard);
                    assert_eq!(output[258], guard);
                    for (i, item) in output[1..258].iter().enumerate() {
                        assert_eq!(*item, cache.dataset_item((start + i) as u64));
                    }
                }
                let mut sentinel = [[17; 8]; 1];
                assert!(init.fill(DATASET_ITEMS, &mut sentinel).is_err());
                assert_eq!(sentinel, [[17; 8]; 1]);
                init.fill(DATASET_ITEMS, &mut []).unwrap();
            }
        }
    }
    #[test]
    fn reciprocal_prefix_matches_runtime_encoder() {
        let mut code = [0xa5; PROGRAM_SIZE];
        let start = t::prologue().len();
        let mut out = Writer {
            code: &mut code,
            pos: start,
            prefetch_mode: 1,
        };
        // Keep the preceding instruction-by-instruction encoder as an
        // independent check of the cached template's RIP and stack offsets.
        for slot in 0..16 {
            out.bytes(&[0x48, 0x8b, 0x0d]);
            out.relative(RECIPROCALS + 8 * slot);
            out.bytes(&[0x48, 0x89, 0x4c, 0x24]);
            out.byte((-8 * (slot as i32 + 1)) as u8);
        }
        assert_eq!(&out.code[start..out.pos], t::stack_reciprocals());
        assert!(out.code[..start].iter().all(|&byte| byte == 0xa5));
        assert!(out.code[out.pos..].iter().all(|&byte| byte == 0xa5));
    }

    #[test]
    fn reused_templates_match_fresh_compilers_across_modes() {
        for light in [false, true] {
            let mut reused = Emitter::new(light);
            for prefetch in 0..4 {
                for hardware in [false, true] {
                    for (step, v2) in [false, true, false, true].into_iter().enumerate() {
                        let mut fresh = Emitter::new(light);
                        reused.prefetch_mode = prefetch;
                        fresh.prefetch_mode = prefetch;
                        let program: Vec<u8> = (0..if v2 { 384 * 8 } else { 256 * 8 })
                            .map(|i| (i * 17 + step * 29 + prefetch as usize) as u8)
                            .collect();
                        let config = || ProgramConfig {
                            read: [0, 3, 4, 7],
                            masks: [0x3ff0000000000000 | step as u64; 2],
                            offset: 123456,
                            v2,
                            light,
                            hardware,
                        };
                        let end = reused.compile(&program, config());
                        assert_eq!(end, fresh.compile(&program, config()));
                        assert_eq!(&reused.code[..end], &fresh.code[..end]);
                        let epilogue = RECIPROCALS - t::epilogue().len();
                        assert_eq!(
                            &reused.code[epilogue..RECIPROCALS],
                            &fresh.code[epilogue..RECIPROCALS]
                        );
                        assert_eq!(
                            &reused.code[ROUNDING..ROUNDING + 16],
                            &fresh.code[ROUNDING..ROUNDING + 16]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn integer_swaps_preserve_other_registers_and_flags() {
        let swap = TYPES.iter().position(|&kind| kind == Kind::Swap).unwrap() as u8;
        let mut program = vec![0u8; 256 * 8];
        for instruction in program.as_chunks_mut::<8>().0 {
            instruction[0] = swap; // Equal operands emit no instructions.
        }
        let mut emitter = Emitter::new(false);
        let mut code = super::super::memory::Executable::new(PROGRAM_SIZE).unwrap();
        for (zen2, prefetch) in [(false, 1), (true, 1), (true, 3)] {
            emitter.zen2 = zen2;
            emitter.prefetch_mode = prefetch;
            for dst in 0..8 {
                for src in 0..8 {
                    program[1] = dst;
                    program[2] = src;
                    emitter.compile(
                        &program,
                        ProgramConfig {
                            read: [0, 2, 4, 6],
                            masks: [0x3ff0000000000000; 2],
                            offset: 0,
                            v2: false,
                            light: false,
                            hardware: true,
                        },
                    );
                    // Full v1 hoists sixteen 12-byte reciprocal loads before the loop.
                    let start = t::prologue().len() + 16 * 12 + t::load().len();
                    let size = if src == dst {
                        0
                    } else if zen2 && prefetch != 3 {
                        9
                    } else {
                        3
                    };
                    let mut bytes = Vec::new();
                    for reg in 4..8 {
                        bytes.extend_from_slice(&[0x41, 0x50 + reg]); // PUSH R12..R15.
                    }
                    for reg in 0..8 {
                        bytes.extend_from_slice(&[0x4c, 0x8b, 0x47 + 8 * reg, 8 * reg]);
                    }
                    bytes.push(0x9c); // Flags before the actual emitted body.
                    bytes.extend_from_slice(&emitter.code[start..start + size]);
                    bytes.extend_from_slice(&[0x9c, 0x58, 0x5a]); // Flags after -> RAX, before -> RDX.
                    for reg in 0..8 {
                        bytes.extend_from_slice(&[0x4c, 0x89, 0x46 + 8 * reg, 8 * reg]);
                    }
                    bytes.extend_from_slice(&[0x48, 0x31, 0xd0]); // Return changed flag bits.
                    for reg in (4..8).rev() {
                        bytes.extend_from_slice(&[0x41, 0x58 + reg]);
                    }
                    bytes.push(0xc3);
                    code.publish(&bytes).unwrap();
                    for input in [
                        [0, 1, u64::MAX, 1 << 63, 1 << 32, 0xdeadbeef01234567, 17, 42],
                        std::array::from_fn(|i| 0x123456789abcdef0u64.rotate_left(i as u32 * 7)),
                    ] {
                        let mut expected = input;
                        expected.swap(dst as usize, src as usize);
                        let mut actual = [0u64; 8];
                        // SAFETY: the wrapper reads/writes exactly eight words through
                        // the System V arguments, preserves callee-saved registers and
                        // balances the stack. Its body is the bounded emitted swap.
                        let flags = unsafe {
                            let function: unsafe extern "sysv64" fn(*const u64, *mut u64) -> u64 =
                                std::mem::transmute(code.entry());
                            function(input.as_ptr(), actual.as_mut_ptr())
                        };
                        assert_eq!(flags, 0, "zen2={zen2} dst={dst} src={src}");
                        assert_eq!(actual, expected, "zen2={zen2} dst={dst} src={src}");
                    }
                }
            }
        }
    }

    #[test]
    fn branch_conditions_preserve_upper_32_bits() {
        let branch = TYPES.iter().position(|&kind| kind == Kind::Branch).unwrap() as u8;
        let swap = TYPES.iter().position(|&kind| kind == Kind::Swap).unwrap() as u8;
        let mut program = vec![0u8; 256 * 8];
        for instruction in program.as_chunks_mut::<8>().0 {
            instruction[0] = swap; // Equal operands emit no instructions.
        }
        let mut emitter = Emitter::new(true);
        let mut code = super::super::memory::Executable::new(PROGRAM_SIZE).unwrap();
        for dst in 0..8 {
            for shift in 8..24 {
                for immediate in [0u32, 0x80000000, u32::MAX] {
                    program[..4].copy_from_slice(&[branch, dst, 0, (shift - 8) << 4]);
                    program[4..8].copy_from_slice(&immediate.to_le_bytes());
                    emitter.compile(
                        &program,
                        ProgramConfig {
                            read: [0, 2, 4, 6],
                            masks: [0x3ff0000000000000; 2],
                            offset: 0,
                            v2: false,
                            light: true,
                            hardware: true,
                        },
                    );
                    // Isolate the first emitted branch. It jumps back to its
                    // own ADD and contains ADD imm32, TEST imm32 and JZ rel8.
                    let start = t::prologue().len() + t::load().len();
                    let body = &emitter.code[start..start + 16];
                    let mut bytes = vec![0x41, 0x50 + dst]; // Preserve R8..R15.
                    bytes.extend_from_slice(&[0x4c, 0x8b, 0xc7 + 8 * dst]); // MOV rN,RDI.
                    bytes.extend_from_slice(body);
                    bytes.extend_from_slice(&[0x49, 0x8b, 0xc0 + dst]); // MOV RAX,rN.
                    bytes.extend_from_slice(&[0x41, 0x58 + dst, 0xc3]);
                    code.publish(&bytes).unwrap();
                    let addend =
                        ((immediate | (1 << shift)) & !(1 << (shift - 1))) as i32 as i64 as u64;
                    let mask = 255u64 << shift;
                    for input in [
                        0,
                        u64::MAX,
                        1 << 63,
                        1 << 32,
                        0u64.wrapping_sub(addend), // Force a taken branch.
                        0xdeadbeef00000000u64.wrapping_sub(addend),
                    ] {
                        let mut expected = input.wrapping_add(addend);
                        if expected & mask == 0 {
                            expected = expected.wrapping_add(addend);
                        }
                        assert_ne!(expected & mask, 0);
                        // SAFETY: bounded emitted ADD/TEST/JZ body; this wrapper
                        // preserves its register, balances the stack and returns
                        // through System V RAX. It accesses no data memory.
                        let actual = unsafe {
                            let function: unsafe extern "sysv64" fn(u64) -> u64 =
                                std::mem::transmute(code.entry());
                            function(input)
                        };
                        assert_eq!(actual, expected, "dst={dst} shift={shift} input={input:x}");
                    }
                }
            }
        }
    }

    #[test]
    fn prologue_constants_remain_aligned_after_relocation() {
        let prologue = t::prologue();
        assert_eq!(prologue.as_ptr() as usize % 64, 0);
        // MOVAPD XMM13/14/15, [RIP+disp32]. Absolute source alignment is not
        // enough: these offsets must remain aligned after copying the fragment
        // to a fresh page, regardless of its placement in the linked binary.
        for modrm in [0x2d, 0x35, 0x3d] {
            let opcode = [0x66, 0x44, 0x0f, 0x28, modrm];
            let positions: Vec<_> = prologue
                .windows(opcode.len())
                .enumerate()
                .filter_map(|(i, bytes)| (bytes == opcode).then_some(i))
                .collect();
            assert_eq!(positions.len(), 1);
            let at = positions[0];
            let displacement = i32::from_le_bytes(prologue[at + 5..at + 9].try_into().unwrap());
            let target = (at as i64 + 9 + i64::from(displacement)) as usize;
            assert!(target
                .checked_add(16)
                .is_some_and(|end| end <= prologue.len()));
            assert_eq!(target % 16, 0, "unaligned relocated constant at {target}");
        }
    }
    #[test]
    fn single_division_reciprocals_match_portable() {
        let check = |d| {
            assert_eq!(
                reciprocal(d),
                crate::randomx_core::superscalar::reciprocal(d)
            )
        };
        check(0);
        check(u32::MAX);
        for bit in 0..32 {
            let value = 1u32 << bit;
            for d in [value - 1, value, value + 1] {
                check(d);
            }
        }
        let mut state = 0x123456789abcdef0u64;
        for _ in 0..65536 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            check(state as u32);
        }
    }

    #[test]
    #[ignore = "256 MiB cache and generated dataset items for both prefetch policies"]
    fn superscalar_prefetch_policies_match_dataset_items() {
        let cache = super::super::new_cache(b"prefetch policy differential", None).unwrap();
        for zen2 in [false, true] {
            let mut emitter = Emitter::new(true);
            emitter.zen2 = zen2;
            emitter.code[PROGRAM_SIZE..].fill(0xcc);
            // Exercise both helper policies in the relocated initializer,
            // regardless of the current CPU's automatic light-mode selection.
            let start = t::dataset_init().len().next_multiple_of(64);
            let end = emitter.dataset_initializer_at(&cache, start, zen2);
            assert_eq!(emitter.code[end - 1], 0xc3);
            assert!(emitter.code[end..].iter().all(|&byte| byte == 0xcc));
            let mut code =
                super::super::memory::Executable::new(end.next_multiple_of(4096)).unwrap();
            code.publish(&emitter.code[..end]).unwrap();
            let initializer = super::super::Initializer {
                cache: &cache,
                code,
            };
            let mut items = [[0; 8]; 257];
            for start in [
                0,
                12345,
                crate::randomx_core::cache::DATASET_ITEMS - items.len(),
            ] {
                initializer.fill(start, &mut items).unwrap();
                for (i, item) in items.iter().enumerate() {
                    assert_eq!(
                        *item,
                        cache.dataset_item((start + i) as u64),
                        "zen2={zen2} item={}",
                        start + i
                    );
                }
            }
        }
    }
    #[test]
    fn worst_case_programs_fit_and_preserve_light_helpers() {
        for light in [false, true] {
            let mut emitter = Emitter::new(light);
            // Compile-only coverage can exercise CPU choices unavailable on
            // this host. Saturate each opcode, including maximum-size v2 paths.
            for bmi2 in [false, true] {
                for zen2 in [false, true] {
                    emitter.bmi2 = bmi2;
                    emitter.zen2 = zen2;
                    for v2 in [false, true] {
                        for hardware in [false, true] {
                            for opcode in 0..=255 {
                                for (dst, src, mode, imm) in
                                    [(5, 4, 0, u32::MAX), (4, 4, 255, 3), (7, 0, 64, 0)]
                                {
                                    let mut program = vec![0; if v2 { 384 * 8 } else { 256 * 8 }];
                                    for instruction in program.as_chunks_mut::<8>().0 {
                                        instruction[..4].copy_from_slice(&[opcode, dst, src, mode]);
                                        instruction[4..].copy_from_slice(&imm.to_le_bytes());
                                    }
                                    emitter.code[PROGRAM_SIZE..].fill(0x5a);
                                    emitter.compile(
                                        &program,
                                        ProgramConfig {
                                            read: [0, 3, 4, 7],
                                            masks: [0x3ff0000000000000; 2],
                                            offset: 33554368,
                                            v2,
                                            light,
                                            hardware,
                                        },
                                    );
                                    assert!(emitter.code[PROGRAM_SIZE..]
                                        .iter()
                                        .all(|&b| b == 0x5a));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
