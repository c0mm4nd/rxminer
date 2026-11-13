// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
// Portable interpreter port; see LICENSE here.
use super::{
    aes::{Aes, Backend},
    blake2b,
    cache::{Cache, CacheError, DATASET_ITEMS},
    float::{self, Rounding},
    superscalar,
};

pub const SCRATCHPAD_SIZE: usize = 2 * 1024 * 1024;
const L1: usize = 16384 - 8;
const L2: usize = 262144 - 8;
const L3: usize = SCRATCHPAD_SIZE - 8;
const DATASET_MASK: u32 = 2147483648 - 64;
const MANTISSA: u64 = (1 << 52) - 1;
const DYNAMIC: u64 = (1 << 56) - 1;

pub trait DatasetSource {
    fn item(&self, index: u64) -> [u64; 8];
    fn prefetch(&self, _index: u64) {}
}
impl DatasetSource for Cache {
    fn item(&self, index: u64) -> [u64; 8] {
        self.dataset_item(index)
    }
}

pub struct Dataset {
    pub(crate) items: super::memory::Memory<[u64; 8]>,
}
impl Dataset {
    pub fn from_memory(items: super::memory::Memory<[u64; 8]>) -> Result<Self, CacheError> {
        if items.len() != DATASET_ITEMS {
            return Err(CacheError::InvalidRange);
        }
        Ok(Self { items })
    }
    pub fn page_size(&self) -> usize {
        self.items.page_size()
    }
    /// Immutable contiguous dataset words, also useful to independent oracles.
    pub fn words(&self) -> &[u64] {
        self.items.as_flattened()
    }
    pub fn new(cache: &Cache) -> Result<Self, CacheError> {
        Self::with_threads(
            cache,
            std::thread::available_parallelism().map_or(1, |n| n.get().min(16)),
        )
    }

    pub fn with_threads(cache: &Cache, threads: usize) -> Result<Self, CacheError> {
        Self::with_control(cache, threads, &std::sync::atomic::AtomicBool::new(false))
    }

    pub fn with_control(
        cache: &Cache,
        threads: usize,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<Self, CacheError> {
        Self::with_workers(cache, threads, stop, |_| Ok(()))
    }

    /// Optional per-worker setup lets the miner apply CPU/NUMA placement before
    /// any dataset items are initialized. The core itself has no OS FFI.
    pub fn with_workers(
        cache: &Cache,
        threads: usize,
        stop: &std::sync::atomic::AtomicBool,
        setup: impl Fn(usize) -> Result<(), CacheError> + Sync,
    ) -> Result<Self, CacheError> {
        Self::with_initializer(threads, stop, setup, |start, output| {
            cache.fill_dataset(start, output)
        })
    }

    /// Allocate the complete dataset and initialize disjoint slices through a
    /// caller-supplied implementation. Worker setup, cancellation and failures
    /// have the same semantics as the portable initializer.
    pub fn with_initializer(
        threads: usize,
        stop: &std::sync::atomic::AtomicBool,
        setup: impl Fn(usize) -> Result<(), CacheError> + Sync,
        fill: impl Fn(usize, &mut [[u64; 8]]) -> Result<(), CacheError> + Sync,
    ) -> Result<Self, CacheError> {
        use std::sync::atomic::Ordering;
        if stop.load(Ordering::Relaxed) {
            return Err(CacheError::Interrupted);
        }
        if threads == 0 {
            return Err(CacheError::InvalidRange);
        }
        let mut items = Vec::new();
        items
            .try_reserve_exact(DATASET_ITEMS)
            .map_err(CacheError::Allocation)?;
        items.resize(DATASET_ITEMS, [0; 8]);
        initialize_dataset(&fill, &mut items, threads, stop, &setup)?;
        Ok(Self {
            items: items.into(),
        })
    }
}
struct InitializationGuard<'a> {
    failed: &'a std::sync::atomic::AtomicBool,
    complete: bool,
}
impl Drop for InitializationGuard<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.failed
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

pub(crate) fn initialize_dataset<T: Send>(
    fill: &(impl Fn(usize, &mut [T]) -> Result<(), CacheError> + Sync),
    items: &mut [T],
    threads: usize,
    stop: &std::sync::atomic::AtomicBool,
    setup: &(impl Fn(usize) -> Result<(), CacheError> + Sync),
) -> Result<(), CacheError> {
    use std::sync::atomic::Ordering;
    // The mutex only hands out disjoint 4 MiB slices. Each worker owns its
    // slice during initialization; no raw pointers or unsafe sharing.
    let chunk_count = items.len().div_ceil(65536);
    let chunks = std::sync::Mutex::new(items.chunks_mut(65536).enumerate());
    let failed = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| -> Result<(), CacheError> {
        let mut workers = Vec::new();
        for i in 0..threads.min(chunk_count) {
            let chunks = &chunks;
            let failed = &failed;
            workers.push(
                std::thread::Builder::new()
                    .name(format!("rust-rx-init-{i}"))
                    .spawn_scoped(scope, move || -> Result<(), CacheError> {
                        let mut guard = InitializationGuard {
                            failed,
                            complete: false,
                        };
                        setup(i)?;
                        loop {
                            if stop.load(Ordering::Relaxed) || failed.load(Ordering::Relaxed) {
                                return Err(CacheError::Interrupted);
                            }
                            let next = chunks.lock().unwrap().next();
                            let Some((index, slice)) = next else {
                                break;
                            };
                            fill(index * 65536, slice)?;
                        }
                        guard.complete = true;
                        Ok(())
                    })
                    .map_err(|error| {
                        failed.store(true, Ordering::Relaxed);
                        CacheError::Thread(error)
                    })?,
            );
        }
        let mut error = None;
        for worker in workers {
            let result = worker.join().unwrap_or_else(|_| {
                Err(CacheError::Thread(std::io::Error::other(
                    "initializer panicked",
                )))
            });
            if let Err(failure) = result {
                if error.is_none() || matches!(error, Some(CacheError::Interrupted)) {
                    error = Some(failure);
                }
            }
        }
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    })
}

impl DatasetSource for Dataset {
    fn item(&self, index: u64) -> [u64; 8] {
        self.items[index as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    AddShift,
    AddMem,
    Sub,
    SubMem,
    Mul,
    MulMem,
    High,
    HighMem,
    SignedHigh,
    SignedHighMem,
    Reciprocal,
    Neg,
    Xor,
    XorMem,
    Ror,
    Rol,
    Swap,
    FloatSwap,
    FloatAdd,
    FloatAddMem,
    FloatSub,
    FloatSubMem,
    FloatScale,
    FloatMul,
    FloatDiv,
    FloatSqrt,
    Branch,
    Round,
    Store,
    Nop,
}
pub(crate) const KINDS: [Kind; 29] = [
    Kind::AddShift,
    Kind::AddMem,
    Kind::Sub,
    Kind::SubMem,
    Kind::Mul,
    Kind::MulMem,
    Kind::High,
    Kind::HighMem,
    Kind::SignedHigh,
    Kind::SignedHighMem,
    Kind::Reciprocal,
    Kind::Neg,
    Kind::Xor,
    Kind::XorMem,
    Kind::Ror,
    Kind::Rol,
    Kind::Swap,
    Kind::FloatSwap,
    Kind::FloatAdd,
    Kind::FloatAddMem,
    Kind::FloatSub,
    Kind::FloatSubMem,
    Kind::FloatScale,
    Kind::FloatMul,
    Kind::FloatDiv,
    Kind::FloatSqrt,
    Kind::Branch,
    Kind::Round,
    Kind::Store,
];
pub(crate) const FREQUENCIES: [u16; 29] = [
    16, 7, 16, 7, 16, 4, 4, 1, 4, 1, 8, 2, 15, 5, 8, 2, 4, 4, 16, 5, 16, 5, 6, 32, 4, 6, 25, 1, 16,
];

#[derive(Clone, Copy)]
pub(crate) struct Instruction {
    pub(crate) kind: Kind,
    pub(crate) dst: usize,
    pub(crate) src: usize,
    pub(crate) imm: u64,
    pub(crate) mask: usize,
    pub(crate) shift: u32,
    pub(crate) target: usize,
}
pub(crate) fn compile(bytes: &[u8], output: &mut Vec<Instruction>) {
    output.clear();
    let mut usage = [0; 8];
    for (i, bytes) in bytes.as_chunks::<8>().0.iter().enumerate() {
        let mut opcode = u16::from(bytes[0]);
        let kind = KINDS[FREQUENCIES
            .iter()
            .position(|&frequency| {
                if opcode < frequency {
                    true
                } else {
                    opcode -= frequency;
                    false
                }
            })
            .unwrap()];
        let raw = u32::from_le_bytes(bytes[4..].try_into().unwrap());
        let mut instr = Instruction {
            kind,
            dst: usize::from(bytes[1] & 7),
            src: usize::from(bytes[2] & 7),
            imm: raw as i32 as i64 as u64,
            mask: if bytes[3] & 3 != 0 { L1 } else { L2 },
            shift: u32::from((bytes[3] >> 2) & 3),
            target: 0,
        };
        use Kind::*;
        match kind {
            AddShift => {
                if instr.dst != 5 {
                    instr.imm = 0;
                }
            }
            AddMem | SubMem | MulMem | HighMem | SignedHighMem | XorMem => {
                if instr.src == instr.dst {
                    instr.src = 8;
                    instr.mask = L3;
                }
            }
            Sub | Mul | Xor | Ror | Rol => {
                if instr.src == instr.dst {
                    instr.src = 8;
                }
            }
            Reciprocal => {
                if let Some(reciprocal) = superscalar::reciprocal(raw) {
                    instr.kind = Mul;
                    instr.src = 8;
                    instr.imm = reciprocal;
                } else {
                    instr.kind = Nop;
                }
            }
            Swap if instr.dst == instr.src => {
                instr.kind = Nop;
            }
            FloatAdd | FloatSub | FloatMul => {
                instr.dst &= 3;
                instr.src &= 3;
            }
            FloatAddMem | FloatSubMem | FloatScale | FloatDiv | FloatSqrt => {
                instr.dst &= 3;
            }
            Branch => {
                instr.target = usage[instr.dst];
                let shift = u32::from(bytes[3] >> 4) + 8;
                instr.imm = (instr.imm | (1 << shift)) & !(1 << (shift - 1));
                instr.mask = 255 << shift;
                usage.fill(i + 1);
            }
            Store if bytes[3] >> 4 >= 14 => {
                instr.mask = L3;
            }
            _ => {}
        }
        if matches!(
            instr.kind,
            AddShift
                | AddMem
                | Sub
                | SubMem
                | Mul
                | MulMem
                | High
                | HighMem
                | SignedHigh
                | SignedHighMem
                | Neg
                | Xor
                | XorMem
                | Ror
                | Rol
                | Swap
        ) {
            usage[instr.dst] = i + 1;
            if instr.kind == Swap {
                usage[instr.src] = i + 1;
            }
        }
        output.push(instr);
    }
}

#[derive(Default)]
#[repr(C, align(16))]
pub(crate) struct Registers {
    pub(crate) r: [u64; 8],
    pub(crate) f: [u64; 8],
    pub(crate) e: [u64; 8],
    pub(crate) a: [u64; 8],
}
#[cfg(target_arch = "x86_64")]
pub(crate) struct RunConfig {
    pub ma: u32,
    pub mx: u32,
    pub read: [usize; 4],
    pub offset: u64,
    pub masks: [u64; 2],
    pub iterations: usize,
    pub v2: bool,
}
#[cfg(target_arch = "x86_64")]
pub(crate) trait ProgramExecutor: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        code: &[Instruction],
        registers: Registers,
        scratch: &mut [u8],
        dataset: &dyn DatasetSource,
        aes: &dyn Backend,
        config: RunConfig,
        rounding: &mut Rounding,
    ) -> Registers;
}
impl Registers {
    fn bytes(&self) -> [u8; 256] {
        let mut output = [0; 256];
        for (chunk, word) in output
            .as_chunks_mut::<8>()
            .0
            .iter_mut()
            .zip(self.r.iter().chain(&self.f).chain(&self.e).chain(&self.a))
        {
            *chunk = word.to_le_bytes();
        }
        output
    }
}

fn load(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn store(bytes: &mut [u8], offset: usize, word: u64) {
    bytes[offset..offset + 8].copy_from_slice(&word.to_le_bytes());
}
fn integers(word: u64) -> [u64; 2] {
    [
        float::from_i32(word as i32),
        float::from_i32((word >> 32) as i32),
    ]
}

fn execute(
    code: &[Instruction],
    reg: &mut Registers,
    scratch: &mut [u8],
    masks: [u64; 2],
    rounding: &mut Rounding,
    v2: bool,
) {
    let mut pc = 0;
    while pc < code.len() {
        let instruction = &code[pc];
        let Instruction {
            kind,
            dst,
            src,
            imm,
            mask,
            shift,
            target,
        } = *instruction;
        let source = if src == 8 { imm } else { reg.r[src] };
        let address_source = if src == 8 { 0 } else { reg.r[src] };
        let address = address_source.wrapping_add(imm) as usize & mask;
        pc += 1;
        use Kind::*;
        match kind {
            AddShift => reg.r[dst] = reg.r[dst].wrapping_add(source << shift).wrapping_add(imm),
            AddMem => reg.r[dst] = reg.r[dst].wrapping_add(load(scratch, address)),
            Sub => reg.r[dst] = reg.r[dst].wrapping_sub(source),
            SubMem => reg.r[dst] = reg.r[dst].wrapping_sub(load(scratch, address)),
            Mul => reg.r[dst] = reg.r[dst].wrapping_mul(source),
            MulMem => reg.r[dst] = reg.r[dst].wrapping_mul(load(scratch, address)),
            High | HighMem => {
                let b = if kind == High {
                    source
                } else {
                    load(scratch, address)
                };
                reg.r[dst] = ((u128::from(reg.r[dst]) * u128::from(b)) >> 64) as u64;
            }
            SignedHigh | SignedHighMem => {
                let b = if kind == SignedHigh {
                    source
                } else {
                    load(scratch, address)
                };
                reg.r[dst] = ((i128::from(reg.r[dst] as i64) * i128::from(b as i64)) >> 64) as u64;
            }
            Neg => reg.r[dst] = reg.r[dst].wrapping_neg(),
            Xor => reg.r[dst] ^= source,
            XorMem => reg.r[dst] ^= load(scratch, address),
            Ror => reg.r[dst] = reg.r[dst].rotate_right(source as u32),
            Rol => reg.r[dst] = reg.r[dst].rotate_left(source as u32),
            Swap => reg.r.swap(dst, src),
            FloatSwap => {
                if dst < 4 {
                    reg.f.swap(2 * dst, 2 * dst + 1);
                } else {
                    reg.e.swap(2 * (dst - 4), 2 * (dst - 4) + 1);
                }
            }
            FloatAdd | FloatSub | FloatAddMem | FloatSubMem => {
                let memory = matches!(kind, FloatAddMem | FloatSubMem);
                let subtract = matches!(kind, FloatSub | FloatSubMem);
                let operand = if memory {
                    integers(load(scratch, address))
                } else {
                    [reg.a[2 * src], reg.a[2 * src + 1]]
                };
                for (lane, b) in operand.into_iter().enumerate() {
                    let a = reg.f[2 * dst + lane];
                    reg.f[2 * dst + lane] = if subtract {
                        float::sub(a, b, *rounding)
                    } else {
                        float::add(a, b, *rounding)
                    };
                }
            }
            FloatScale => {
                for lane in 0..2 {
                    reg.f[2 * dst + lane] ^= 0x80f0000000000000;
                }
            }
            FloatMul => {
                for lane in 0..2 {
                    reg.e[2 * dst + lane] =
                        float::mul(reg.e[2 * dst + lane], reg.a[2 * src + lane], *rounding);
                }
            }
            FloatDiv => {
                let operand = integers(load(scratch, address));
                for lane in 0..2 {
                    reg.e[2 * dst + lane] = float::div(
                        reg.e[2 * dst + lane],
                        (operand[lane] & DYNAMIC) | masks[lane],
                        *rounding,
                    );
                }
            }
            FloatSqrt => {
                for lane in 0..2 {
                    reg.e[2 * dst + lane] = float::sqrt(reg.e[2 * dst + lane], *rounding);
                }
            }
            Branch => {
                reg.r[dst] = reg.r[dst].wrapping_add(imm);
                if reg.r[dst] & mask as u64 == 0 {
                    pc = target;
                }
            }
            Round => {
                let value = source.rotate_right(imm as u32 & 63);
                if !v2 || value & 60 == 0 {
                    *rounding = Rounding::from_bits(value);
                }
            }
            Store => store(
                scratch,
                reg.r[dst].wrapping_add(imm) as usize & mask,
                source,
            ),
            Nop => {}
            Reciprocal => unreachable!("reciprocals are compiled as multiplication"),
        }
    }
}

/// Portable v1/v2 reference VM, using explicit software floating-point rounding.
/// It is independent of the native library and preserves the host FP environment.
pub struct Vm {
    scratch: super::memory::Memory<u8>,
    aes: Box<dyn Backend>,
    code: Vec<Instruction>,
    pending: Option<[u8; 64]>,
    #[cfg(target_arch = "x86_64")]
    executor: Option<Box<dyn ProgramExecutor>>,
}
impl Vm {
    pub fn new() -> Result<Self, CacheError> {
        let mut scratch = Vec::new();
        scratch
            .try_reserve_exact(SCRATCHPAD_SIZE)
            .map_err(CacheError::Allocation)?;
        scratch.resize(SCRATCHPAD_SIZE, 0);
        Self::with_backends(scratch.into(), Box::new(Aes::default()))
    }
    pub fn with_backends(
        scratch: super::memory::Memory<u8>,
        aes: Box<dyn Backend>,
    ) -> Result<Self, CacheError> {
        if scratch.len() != SCRATCHPAD_SIZE {
            return Err(CacheError::InvalidRange);
        }
        Ok(Self {
            scratch,
            aes,
            code: Vec::with_capacity(384),
            pending: None,
            #[cfg(target_arch = "x86_64")]
            executor: None,
        })
    }

    #[cfg(target_arch = "x86_64")]
    pub(crate) fn with_executor(mut self, executor: Box<dyn ProgramExecutor>) -> Self {
        self.executor = Some(executor);
        self
    }
    pub fn page_size(&self) -> usize {
        self.scratch.page_size()
    }
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn hash(&mut self, dataset: &impl DatasetSource, input: &[u8], v2: bool) -> [u8; 32] {
        self.first(input);
        self.last(dataset, v2)
    }

    pub fn first(&mut self, input: &[u8]) {
        assert!(self.pending.is_none(), "hash already pending");
        let mut seed = blake2b::hash_512(input);
        self.aes.fill(&mut seed, &mut self.scratch);
        self.pending = Some(seed);
    }

    pub fn next(&mut self, dataset: &impl DatasetSource, input: &[u8], v2: bool) -> [u8; 32] {
        self.finish(dataset, v2, Some(blake2b::hash_512(input)))
    }

    pub fn last(&mut self, dataset: &impl DatasetSource, v2: bool) -> [u8; 32] {
        self.finish(dataset, v2, None)
    }
    fn finish(
        &mut self,
        dataset: &impl DatasetSource,
        v2: bool,
        mut next: Option<[u8; 64]>,
    ) -> [u8; 32] {
        let mut seed = self.pending.take().expect("no hash pending");
        let mut rounding = Rounding::Nearest;
        for _ in 0..7 {
            let registers = self.run(dataset, &seed, v2, &mut rounding);
            seed = blake2b::hash_512(&registers.bytes());
        }
        let mut registers = self.run(dataset, &seed, v2, &mut rounding);
        let hash = self.aes.finish(&mut self.scratch, next.as_mut());
        self.pending = next;
        for (word, bytes) in registers.a.iter_mut().zip(hash.as_chunks::<8>().0) {
            *word = u64::from_le_bytes(*bytes);
        }
        let mut output = [0; 32];
        blake2b::hash(&registers.bytes(), &mut output).unwrap();
        output
    }

    fn run(
        &mut self,
        dataset: &impl DatasetSource,
        seed: &[u8; 64],
        v2: bool,
        rounding: &mut Rounding,
    ) -> Registers {
        let mut program = [0; 128 + 384 * 8];
        let size = 128 + if v2 { 384 } else { 256 } * 8;
        self.aes.program(seed, &mut program[..size]);
        self.run_program(dataset, &program[..size], v2, rounding, 2048)
    }
    #[cfg(all(
        test,
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )
    ))]
    pub(crate) fn trace_program(
        &mut self,
        dataset: &impl DatasetSource,
        program: &[u8],
        v2: bool,
        iterations: usize,
    ) -> ([u8; 256], Vec<u8>) {
        let result = self.run_program(dataset, program, v2, &mut Rounding::Nearest, iterations);
        (result.bytes(), self.scratch.to_vec())
    }
    #[cfg(all(
        test,
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )
    ))]
    pub(crate) fn trace_chain_program(
        &mut self,
        dataset: &impl DatasetSource,
        program: &[u8],
        v2: bool,
        rounding: &mut Rounding,
    ) -> ([u8; 256], Vec<u8>) {
        let result = self.run_program(dataset, program, v2, rounding, 2048);
        (result.bytes(), self.scratch.to_vec())
    }
    fn run_program(
        &mut self,
        dataset: &impl DatasetSource,
        program: &[u8],
        v2: bool,
        rounding: &mut Rounding,
        iterations: usize,
    ) -> Registers {
        let size = program.len();
        let entropy = |i| load(program, i * 8);
        let mut registers = Registers::default();
        for i in 0..8 {
            let bits = entropy(i);
            registers.a[i] = (((bits >> 59) + 1023) << 52) | (bits & MANTISSA);
        }
        let mut ma = entropy(8) as u32 & DATASET_MASK;
        let mut mx = entropy(10) as u32;
        let read: [usize; 4] = std::array::from_fn(|i| 2 * i + ((entropy(12) >> i) & 1) as usize);
        let dataset_offset = (entropy(13) % 524288) * 64;
        let masks = [entropy(14), entropy(15)]
            .map(|bits| (bits & ((1 << 22) - 1)) | ((0x300 | ((bits >> 60) << 4)) << 52));
        compile(&program[128..size], &mut self.code);
        #[cfg(target_arch = "x86_64")]
        if let Some(executor) = &mut self.executor {
            return executor.run(
                &self.code,
                registers,
                &mut self.scratch,
                dataset,
                &*self.aes,
                RunConfig {
                    ma,
                    mx,
                    read,
                    offset: dataset_offset,
                    masks,
                    iterations,
                    v2,
                },
                rounding,
            );
        }
        let mut sp0 = mx as usize;
        let mut sp1 = ma as usize;
        for _ in 0..iterations {
            let mix = registers.r[read[0]] ^ registers.r[read[1]];
            sp0 = (sp0 ^ mix as usize) & (SCRATCHPAD_SIZE - 64);
            sp1 = (sp1 ^ (mix >> 32) as usize) & (SCRATCHPAD_SIZE - 64);
            for i in 0..8 {
                registers.r[i] ^= load(&self.scratch, sp0 + 8 * i);
            }
            for i in 0..4 {
                registers.f[2 * i..2 * i + 2]
                    .copy_from_slice(&integers(load(&self.scratch, sp1 + 8 * i)));
                let values = integers(load(&self.scratch, sp1 + 8 * (4 + i)));
                for lane in 0..2 {
                    registers.e[2 * i + lane] = (values[lane] & DYNAMIC) | masks[lane];
                }
            }
            execute(
                &self.code,
                &mut registers,
                &mut self.scratch,
                masks,
                rounding,
                v2,
            );
            let address = dataset_offset + u64::from(ma & DATASET_MASK);
            let mix = (registers.r[read[2]] ^ registers.r[read[3]]) as u32;
            if v2 {
                ma ^= mix;
            } else {
                mx ^= mix;
            }
            for (register, value) in registers.r.iter_mut().zip(dataset.item(address / 64)) {
                *register ^= value;
            }
            std::mem::swap(&mut mx, &mut ma);
            for i in 0..8 {
                store(&mut self.scratch, sp1 + 8 * i, registers.r[i]);
            }
            if v2 {
                self.aes.mix(&mut registers.f, &registers.e);
            } else {
                for i in 0..8 {
                    registers.f[i] ^= registers.e[i];
                }
            }
            for i in 0..8 {
                store(&mut self.scratch, sp0 + 8 * i, registers.f[i]);
            }
            sp0 = 0;
            sp1 = 0;
        }
        registers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "256 MiB cache and full portable v1/v2 hashes; use an optimized build"]
    fn upstream_full_hashes() {
        let cache = Cache::new(b"test key 000").unwrap();
        let stopped = std::sync::atomic::AtomicBool::new(true);
        assert!(matches!(
            Dataset::with_control(&cache, 8, &stopped),
            Err(CacheError::Interrupted)
        ));
        let running = std::sync::atomic::AtomicBool::new(false);
        let mut prefix = vec![[0; 8]; 65537];
        initialize_dataset(
            &|start, output| cache.fill_dataset(start, output),
            &mut prefix,
            2,
            &running,
            &|_| Ok(()),
        )
        .unwrap();
        for index in [0, 65535, 65536] {
            assert_eq!(prefix[index], cache.dataset_item(index as u64));
        }
        let error = initialize_dataset(
            &|start, output| cache.fill_dataset(start, output),
            &mut prefix,
            2,
            &running,
            &|i| {
                if i == 0 {
                    Err(CacheError::Thread(std::io::Error::other(
                        "injected setup failure",
                    )))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("injected setup failure"));
        let mut vm = Vm::new().unwrap();
        for (v2, expected) in [
            (
                false,
                "639183aae1bf4c9a35884cb46b09cad9175f04efd7684e7262a0ac1c2f0b4e3f",
            ),
            (
                true,
                "22ec6b861b3eb23686b2efbad69513c967ecfce80983df66c9c5b4fbfb4cdb6f",
            ),
        ] {
            let hash = vm.hash(&cache, b"This is a test", v2);
            let actual: String = hash.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(actual, expected, "v2={v2}");
        }
        // Reuse the VM and switch versions while the next input is pending.
        vm.first(b"This is a test");
        let v1 = vm.next(&cache, b"This is a test", false);
        let v2 = vm.next(&cache, b"Lorem ipsum dolor sit amet", true);
        let last = vm.last(&cache, false);
        for (actual, expected) in [
            (
                v1,
                "639183aae1bf4c9a35884cb46b09cad9175f04efd7684e7262a0ac1c2f0b4e3f",
            ),
            (
                v2,
                "22ec6b861b3eb23686b2efbad69513c967ecfce80983df66c9c5b4fbfb4cdb6f",
            ),
            (
                last,
                "300a0adb47603dedb42228ccb2b211104f4da45af709cd7547cd049e9489c969",
            ),
        ] {
            assert_eq!(
                actual
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                expected
            );
        }
    }

    #[test]
    #[ignore = "builds the complete 2080 MiB dataset with 8 workers; use an optimized build"]
    fn full_dataset_matches_light() {
        let cache = Cache::new(b"test key 000").unwrap();
        let dataset = Dataset::with_threads(&cache, 8).unwrap();
        for index in [
            0,
            1,
            65535,
            65536,
            10000000,
            20000000,
            30000000,
            DATASET_ITEMS - 1,
        ] {
            assert_eq!(dataset.item(index as u64), cache.dataset_item(index as u64));
        }
        let mut vm = Vm::new().unwrap();
        for v2 in [false, true] {
            assert_eq!(
                vm.hash(&dataset, b"This is a test", v2),
                vm.hash(&cache, b"This is a test", v2)
            );
        }
    }
}
