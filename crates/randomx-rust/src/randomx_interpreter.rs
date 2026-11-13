// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 RxMiner contributors
//! x86-64 interpreter using SSE2 floating point, without generating executable code.
use crate::randomx_core::{
    aes::Backend,
    float::Rounding,
    vm::{
        DatasetSource, Instruction, Kind, ProgramExecutor, Registers, RunConfig, SCRATCHPAD_SIZE,
    },
};
use std::arch::{asm, x86_64::*};
struct FpGuard(u32);
impl FpGuard {
    fn new(rounding: Rounding) -> Self {
        let mut old = 0;
        let initial = 0x9fc0u32 | ((rounding as u32) << 13);
        // SAFETY: full control words, SSE2 baseline; all FP exceptions masked.
        unsafe {
            asm!("stmxcsr [{}]","ldmxcsr [{}]",in(reg)&mut old,in(reg)&initial,options(nostack,preserves_flags));
        }
        Self(old)
    }
}
impl Drop for FpGuard {
    fn drop(&mut self) {
        // SAFETY: exact saved valid caller state, including exception flags.
        unsafe {
            asm!("ldmxcsr [{}]",in(reg)&self.0,options(nostack,preserves_flags));
        }
    }
}
#[derive(Clone, Copy)]
#[repr(C)]
struct Decoded {
    dst: *mut u64,
    src: *const u64,
    imm: u64,
    mask: u32,
    target: u16,
    shift: u8,
    kind: Kind,
}
#[derive(Default)]
pub(crate) struct Interpreter {
    decoded: Vec<Decoded>,
    hardware_aes: bool,
}
static ZERO: u64 = 0;
// SAFETY: decoded pointers are accessed only during an exclusive run borrow.
// Every new run clears them before use, including after an unwound prior run.
unsafe impl Send for Interpreter {}
// SAFETY: shared references cannot execute, dereference or change decoded state.
unsafe impl Sync for Interpreter {}
impl ProgramExecutor for Interpreter {
    fn run(
        &mut self,
        code: &[Instruction],
        reg: Registers,
        scratch: &mut [u8],
        dataset: &dyn DatasetSource,
        aes: &dyn Backend,
        c: RunConfig,
        rounding: &mut Rounding,
    ) -> Registers {
        if c.v2 {
            self.run_version::<true>(code, reg, scratch, dataset, aes, c, rounding)
        } else {
            self.run_version::<false>(code, reg, scratch, dataset, aes, c, rounding)
        }
    }
}
impl Interpreter {
    pub(crate) fn new(hardware_aes: bool) -> Self {
        Self {
            decoded: Vec::new(),
            hardware_aes: hardware_aes && std::is_x86_feature_detected!("aes"),
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn run_version<const V2: bool>(
        &mut self,
        code: &[Instruction],
        mut reg: Registers,
        scratch: &mut [u8],
        dataset: &dyn DatasetSource,
        aes: &dyn Backend,
        c: RunConfig,
        rounding: &mut Rounding,
    ) -> Registers {
        assert_eq!(scratch.len(), SCRATCHPAD_SIZE);
        let _guard = FpGuard::new(*rounding);
        self.decoded.clear();
        let r = reg.r.as_mut_ptr();
        let f = reg.f.as_mut_ptr();
        let e = reg.e.as_mut_ptr();
        let a = reg.a.as_ptr();
        // SAFETY: core decoder bounds all register indices, shifts, scratchpad masks
        // and branch targets. Every pointer below addresses a live register or
        // immutable decoded immediate; all are used only before this run returns.
        unsafe {
            for i in code {
                use Kind::*;
                let mut d = Decoded {
                    kind: i.kind,
                    dst: r.add(i.dst),
                    src: if i.src == 8 {
                        std::ptr::null()
                    } else {
                        r.add(i.src)
                    },
                    imm: i.imm,
                    mask: i.mask as u32,
                    shift: i.shift as u8,
                    target: i.target as u16,
                };
                match i.kind {
                    AddMem | SubMem | MulMem | HighMem | SignedHighMem | XorMem => {
                        if i.src == 8 {
                            d.src = &ZERO;
                        }
                    }
                    FloatAdd | FloatSub => {
                        d.dst = f.add(i.dst * 2);
                        d.src = a.add(i.src * 2);
                    }
                    FloatAddMem | FloatSubMem | FloatScale => {
                        d.dst = f.add(i.dst * 2);
                    }
                    FloatMul => {
                        d.dst = e.add(i.dst * 2);
                        d.src = a.add(i.src * 2);
                    }
                    FloatDiv | FloatSqrt => {
                        d.dst = e.add(i.dst * 2);
                    }
                    FloatSwap => {
                        d.dst = if i.dst < 4 {
                            f.add(i.dst * 2)
                        } else {
                            e.add((i.dst - 4) * 2)
                        };
                    }
                    _ => {}
                }
                self.decoded.push(d);
            }
            // Resolve immediate operands only after Vec growth has finished.
            // Keep their loads in the compact decoded array, avoiding accesses
            // to the larger portable instruction array inside the hot loop.
            for d in &mut self.decoded {
                if d.src.is_null() {
                    d.src = &raw const d.imm;
                }
            }
            let scratch = scratch.as_mut_ptr();
            let masks = _mm_loadu_pd(c.masks.as_ptr().cast());
            let dynamic = _mm_castsi128_pd(_mm_set1_epi64x(0x00ffffffffffffff));
            let mut ma = c.ma;
            let mut mx = c.mx;
            let mut sp0 = mx as usize;
            let mut sp1 = ma as usize;
            for _ in 0..c.iterations {
                let mix = *r.add(c.read[0]) ^ *r.add(c.read[1]);
                sp0 = (sp0 ^ mix as usize) & (SCRATCHPAD_SIZE - 64);
                sp1 = (sp1 ^ (mix >> 32) as usize) & (SCRATCHPAD_SIZE - 64);
                for i in 0..8 {
                    *r.add(i) ^= scratch.add(sp0 + 8 * i).cast::<u64>().read_unaligned();
                }
                for i in 0..4 {
                    let x = integers(scratch.add(sp1 + 8 * i));
                    let y = integers(scratch.add(sp1 + 8 * (4 + i)));
                    _mm_storeu_pd(f.add(2 * i).cast(), x);
                    _mm_storeu_pd(
                        e.add(2 * i).cast(),
                        _mm_or_pd(_mm_and_pd(y, dynamic), masks),
                    );
                }
                execute::<V2>(&self.decoded, scratch, masks, dynamic, rounding);
                let address = c.offset + u64::from(ma & 0x7fffffc0);
                let mix = (*r.add(c.read[2]) ^ *r.add(c.read[3])) as u32;
                if V2 {
                    ma ^= mix
                } else {
                    mx ^= mix
                }
                dataset
                    .prefetch((c.offset + u64::from((if V2 { ma } else { mx }) & 0x7fffffc0)) / 64);
                let item = dataset.item(address / 64);
                for (i, v) in item.into_iter().enumerate() {
                    *r.add(i) ^= v;
                }
                std::mem::swap(&mut mx, &mut ma);
                for i in 0..8 {
                    scratch
                        .add(sp1 + 8 * i)
                        .cast::<u64>()
                        .write_unaligned(*r.add(i));
                }
                if V2 {
                    // Scoped reborrows from the disjoint f/e raw pointers;
                    // the backend cannot retain these register references.
                    let fv = &mut *f.cast::<[u64; 8]>();
                    let ev = &*e.cast::<[u64; 8]>();
                    if self.hardware_aes {
                        crate::randomx_jit::aes::mix(fv, ev);
                    } else {
                        aes.mix(fv, ev);
                    }
                } else {
                    for i in 0..4 {
                        _mm_storeu_pd(
                            f.add(2 * i).cast(),
                            _mm_xor_pd(
                                _mm_loadu_pd(f.add(2 * i).cast()),
                                _mm_loadu_pd(e.add(2 * i).cast()),
                            ),
                        );
                    }
                }
                std::ptr::copy_nonoverlapping(f.cast::<u8>(), scratch.add(sp0), 64);
                sp0 = 0;
                sp1 = 0;
            }
        }
        self.decoded.clear();
        reg
    }
}
#[inline(always)]
unsafe fn integers(p: *const u8) -> __m128d {
    // SAFETY: caller provides eight initialized bytes; conversion is exact for i32.
    unsafe { _mm_cvtepi32_pd(_mm_cvtsi64_si128(p.cast::<i64>().read_unaligned())) }
}
macro_rules! fp_binary {($name:ident,$op:literal)=>{
 #[inline(always)] unsafe fn $name(mut a:__m128d,b:__m128d)->__m128d {
  // SAFETY: SSE2 baseline, explicit asm keeps rounding-sensitive operations
  // ordered with MXCSR writes and prevents constant folding under nearest mode.
  unsafe{asm!(concat!($op," {a}, {b}"),a=inout(xmm_reg)a,b=in(xmm_reg)b,options(nostack,nomem,preserves_flags));}a
 }
};}
macro_rules! fp_memory {($name:ident,$op:literal)=>{
 #[inline(always)] unsafe fn $name(mut a:__m128d,b:*const u64)->__m128d {
  // SAFETY: source addresses a full aligned pair of initialized registers.
  // Side-effecting asm retains dynamic rounding semantics.
  unsafe{asm!(concat!($op," {a}, [{b}]"),a=inout(xmm_reg)a,b=in(reg)b,options(nostack,readonly,preserves_flags));}a
 }
};}
fp_memory!(add_memory, "addpd");
fp_memory!(sub_memory, "subpd");
fp_memory!(mul_memory, "mulpd");
fp_binary!(add, "addpd");
fp_binary!(sub, "subpd");
fp_binary!(div, "divpd");
#[inline(never)]
unsafe fn execute<const V2: bool>(
    code: &[Decoded],
    scratch: *mut u8,
    masks: __m128d,
    dynamic: __m128d,
    rounding: &mut Rounding,
) {
    // SAFETY: all decoded operands and branch/scratch indices were produced by
    // the validated core decoder; registers and complete scratchpad remain live.
    unsafe {
        // Give this dispatch function consistent cache-line alignment even
        // when changes elsewhere alter linker placement. Padding runs once
        // per program iteration, outside the instruction-dispatch loop.
        asm!(".p2align 6", options(nomem, nostack, preserves_flags));
        let mut pc = 0;
        while pc < code.len() {
            let i = code.get_unchecked(pc);
            pc += 1;
            let d = i.dst;
            let s = i.src;
            use Kind::*;
            macro_rules! memory {
                () => {
                    scratch.add((*s).wrapping_add(i.imm) as usize & i.mask as usize)
                };
            }
            macro_rules! memword {
                () => {
                    memory!().cast::<u64>().read_unaligned()
                };
            }
            macro_rules! fp {
                ($op:ident,$value:expr) => {
                    _mm_storeu_pd(d.cast(), $op(_mm_loadu_pd(d.cast()), $value))
                };
            }
            match i.kind {
                AddShift => *d = (*d).wrapping_add(*s << i.shift).wrapping_add(i.imm),
                AddMem => *d = (*d).wrapping_add(memword!()),
                Sub => *d = (*d).wrapping_sub(*s),
                SubMem => *d = (*d).wrapping_sub(memword!()),
                Mul => *d = (*d).wrapping_mul(*s),
                MulMem => *d = (*d).wrapping_mul(memword!()),
                High => *d = (((*d as u128) * (*s as u128)) >> 64) as u64,
                HighMem => *d = (((*d as u128) * (memword!() as u128)) >> 64) as u64,
                SignedHigh => *d = (((*d as i64 as i128) * (*s as i64 as i128)) >> 64) as u64,
                SignedHighMem => {
                    *d = (((*d as i64 as i128) * (memword!() as i64 as i128)) >> 64) as u64
                }
                Neg => *d = (*d).wrapping_neg(),
                Xor => *d ^= *s,
                XorMem => *d ^= memword!(),
                Ror => *d = (*d).rotate_right(*s as u32),
                Rol => *d = (*d).rotate_left(*s as u32),
                Swap => std::ptr::swap(d, s.cast_mut()),
                FloatSwap => {
                    let x = _mm_loadu_pd(d.cast());
                    _mm_storeu_pd(d.cast(), _mm_shuffle_pd::<1>(x, x));
                }
                FloatAdd => fp!(add_memory, s),
                FloatSub => fp!(sub_memory, s),
                FloatAddMem => fp!(add, integers(memory!())),
                FloatSubMem => fp!(sub, integers(memory!())),
                FloatScale => _mm_storeu_pd(
                    d.cast(),
                    _mm_xor_pd(
                        _mm_loadu_pd(d.cast()),
                        _mm_castsi128_pd(_mm_set1_epi64x(0x80f0000000000000u64 as i64)),
                    ),
                ),
                FloatMul => fp!(mul_memory, s),
                FloatDiv => fp!(
                    div,
                    _mm_or_pd(_mm_and_pd(integers(memory!()), dynamic), masks)
                ),
                FloatSqrt => {
                    let mut x = _mm_loadu_pd(d.cast());
                    asm!("sqrtpd {x}, {x}",x=inout(xmm_reg)x,options(nostack,nomem,preserves_flags));
                    _mm_storeu_pd(d.cast(), x);
                }
                Branch => {
                    *d = (*d).wrapping_add(i.imm);
                    if *d & i.mask as u64 == 0 {
                        pc = i.target as usize;
                    }
                }
                Round => {
                    let value = (*s).rotate_right(i.imm as u32 & 63);
                    if !V2 || value & 60 == 0 {
                        let next = Rounding::from_bits(value);
                        if next != *rounding {
                            let csr = 0x9fc0u32 | ((next as u32) << 13);
                            asm!("ldmxcsr [{}]",in(reg)&csr,options(nostack,preserves_flags));
                            *rounding = next;
                        }
                    }
                }
                Store => scratch
                    .add((*d).wrapping_add(i.imm) as usize & i.mask as usize)
                    .cast::<u64>()
                    .write_unaligned(*s),
                Nop => {}
                Reciprocal => unreachable!("core decoder lowers reciprocals"),
            }
        }
    }
}

/// Fixed-code integer interpreter. Conditional dispatch avoids a many-way
/// indirect branch in the hot superscalar loop; this allocates no executable memory.
pub(crate) fn dataset_item(cache: &crate::randomx_core::cache::Cache, item: u64) -> [u64; 8] {
    use crate::randomx_core::cache::{ADDS, CACHE_SIZE};
    let words = cache.words();
    assert_eq!(words.len(), CACHE_SIZE / 8);
    let first = item.wrapping_add(1).wrapping_mul(6364136223846793005);
    let mut regs = ADDS.map(|v| v ^ first);
    let mut address = item;
    for program in &cache.programs {
        let offset = (address as usize & (CACHE_SIZE / 64 - 1)) * 8;
        let (code, reciprocals) = program.packed();
        // SAFETY: packed instructions have validated register/reciprocal indices and
        // eight-byte repr(C) layout. Owned cache backing and register array stay live.
        unsafe {
            _mm_prefetch(words.as_ptr().add(offset).cast(), _MM_HINT_NTA);
            asm!(
             ".p2align 6", "2:",
             "movzx {dst:e}, byte ptr [{code}+5]",
             "movzx ecx, byte ptr [{code}+4]", "cmp ecx, 6", "ja 9f", "cmp ecx, 3", "ja 4f",
             "movzx edx, byte ptr [{code}+6]", "mov rax, [{registers}+rdx*8]",
             "cmp ecx, 2", "je 7f", "cmp ecx, 3", "je 8f", "test ecx, ecx", "je 6f",
             "xor [{registers}+{dst}*8], rax", "jmp 20f",
             "6:", "sub [{registers}+{dst}*8], rax", "jmp 20f",
             "7:", "mov cl, byte ptr [{code}+7]", "shl rax, cl", "add [{registers}+{dst}*8], rax", "jmp 20f",
             "8:", "imul rax, [{registers}+{dst}*8]", "mov [{registers}+{dst}*8], rax", "jmp 20f",
             "4:", "cmp ecx, 4", "je 5f",
             "movsxd rax, dword ptr [{code}]", "cmp ecx, 5", "je 12f",
             "xor [{registers}+{dst}*8], rax", "jmp 20f",
             "12:", "add [{registers}+{dst}*8], rax", "jmp 20f",
             "5:", "mov cl, byte ptr [{code}+7]", "ror qword ptr [{registers}+{dst}*8], cl", "jmp 20f",
             "9:", "cmp ecx, 9", "je 14f",
             "movzx edx, byte ptr [{code}+6]", "mov rax, [{registers}+rdx*8]",
             "cmp ecx, 7", "je 13f", "imul qword ptr [{registers}+{dst}*8]", "mov [{registers}+{dst}*8], rdx", "jmp 20f",
             "13:", "mul qword ptr [{registers}+{dst}*8]", "mov [{registers}+{dst}*8], rdx", "jmp 20f",
             "14:", "mov eax, dword ptr [{code}]", "mov rax, [{reciprocals}+rax*8]", "imul rax, [{registers}+{dst}*8]", "mov [{registers}+{dst}*8], rax",
             "20:", "add {code}, 8", "cmp {code}, {end}", "jb 2b",
             code=inout(reg)code.as_ptr()=>_,end=in(reg)code.as_ptr().add(code.len()),
             registers=in(reg)regs.as_mut_ptr(),reciprocals=in(reg)reciprocals.as_ptr(),dst=out(reg)_,
             out("rax")_,out("rcx")_,out("rdx")_,options(nostack)
            );
        }
        for i in 0..8 {
            regs[i] ^= words[offset + i];
        }
        address = regs[program.address_register()];
    }
    regs
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "256 MiB cache, every interpreter instruction and full chains"]
    fn instructions_chains_and_fp_state() {
        use crate::randomx_core::{aes::Aes, blake2b, cache::Cache, vm};
        assert_eq!(
            std::mem::size_of::<crate::randomx_core::superscalar::Packed>(),
            8
        );
        assert_eq!(std::mem::size_of::<Decoded>(), 32);
        let cache = Cache::new(b"test key 000").unwrap();
        for i in (0..1024)
            .map(|i| i * 32771u64)
            .chain([u32::MAX as u64, u64::MAX])
        {
            assert_eq!(
                dataset_item(&cache, i),
                cache.dataset_item(i),
                "dataset item {i}"
            );
        }
        let _restore = FpGuard::new(Rounding::Nearest);
        let caller = 0x5fa5u32;
        // SAFETY: valid MXCSR with masked exceptions, upward rounding and flags.
        unsafe {
            asm!("ldmxcsr [{}]",in(reg)&caller,options(nostack,preserves_flags));
        }
        let mut opcode = 0u16;
        for frequency in vm::FREQUENCIES
            .into_iter()
            .chain(std::iter::repeat_n(1, 32))
        {
            for (v2, hardware) in [(false, false), (true, false), (false, true), (true, true)] {
                let mut program = vec![0; 128 + if v2 { 384 * 8 } else { 256 * 8 }];
                for (i, bytes) in program[128..].as_chunks_mut::<8>().0.iter_mut().enumerate() {
                    bytes.copy_from_slice(&[
                        opcode as u8,
                        (i % 8) as u8,
                        ((i * 3 + 1) % 8) as u8,
                        0x65,
                        0x71,
                        0x32,
                        0x91,
                        0xa3,
                    ]);
                }
                if opcode >= 256 {
                    Aes::default().program(&[opcode as u8; 64], &mut program);
                }
                let mut portable = vm::Vm::new().unwrap();
                let mut native = vm::Vm::new()
                    .unwrap()
                    .with_executor(Box::new(Interpreter::new(hardware)));
                portable.first(b"This is a test");
                native.first(b"This is a test");
                assert_eq!(
                    native.trace_program(&cache, &program, v2, 3),
                    portable.trace_program(&cache, &program, v2, 3),
                    "opcode={opcode} v2={v2}"
                );
                let mut actual = 0u32;
                // SAFETY: write the current MXCSR to a full stack control word.
                unsafe {
                    asm!("stmxcsr [{}]",in(reg)&mut actual,options(nostack,preserves_flags));
                }
                assert_eq!(actual, caller);
            }
            opcode += frequency;
        }
        for (v2, hardware) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut portable = vm::Vm::new().unwrap();
            let mut native = vm::Vm::new()
                .unwrap()
                .with_executor(Box::new(Interpreter::new(hardware)));
            portable.first(b"This is a test");
            native.first(b"This is a test");
            let mut seed = blake2b::hash_512(b"This is a test");
            Aes::default().fill(&mut seed, &mut vec![0; SCRATCHPAD_SIZE]);
            let mut ra = Rounding::Nearest;
            let mut rb = ra;
            for stage in 0..8 {
                let mut program = vec![0; 128 + if v2 { 384 * 8 } else { 256 * 8 }];
                Aes::default().program(&seed, &mut program);
                let expected = portable.trace_chain_program(&cache, &program, v2, &mut ra);
                let actual = native.trace_chain_program(&cache, &program, v2, &mut rb);
                assert_eq!(actual, expected, "stage={stage} v2={v2}");
                assert_eq!(ra, rb);
                seed = blake2b::hash_512(&expected.0);
            }
        }
    }
}
