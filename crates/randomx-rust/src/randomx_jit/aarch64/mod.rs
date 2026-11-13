// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2020, tevador <tevador@gmail.com>
// Copyright (c) 2019-2021, SChernykh <https://github.com/SChernykh>
// Copyright (c) 2019-2021, XMRig <https://github.com/xmrig>
// Copyright (c) 2026 RxMiner contributors
//! Rust-owned ARM64 JIT, ARMv8 AES and dataset initializer. The portable core
//! remains an independent, unsafe-free reference. No native RandomX FFI.
pub(crate) mod aes {
    pub(crate) use crate::randomx_aes_arm::HardwareAes;
}
pub(crate) mod argon2 {
    use crate::randomx_core::cache::{fill_memory, Cache, CacheError, CACHE_SIZE};
    pub(crate) fn available(force: Option<u32>) -> bool {
        matches!(force, None | Some(0) | Some(96))
    }
    pub(super) fn cache(
        key: &[u8],
        force: Option<u32>,
        policy: crate::randomx_memory::Policy,
    ) -> Result<Cache, CacheError> {
        if !available(force) {
            return Err(CacheError::Backend(std::io::Error::other(
                "Argon2 backend unavailable",
            )));
        }
        u32::try_from(key.len()).map_err(|_| CacheError::KeyTooLong)?;
        let memory = crate::randomx_memory::Mapped::<[u64; 128]>::new(CACHE_SIZE / 1024, policy)
            .map_err(CacheError::Backend)?;
        Cache::with_memory(key, memory.into_memory(), fill_memory)
    }
}
mod emitter;
#[path = "../memory.rs"]
mod memory;
mod tables;
mod templates;
use crate::randomx_core::{aes::Aes, blake2b, cache::Cache, vm::Dataset};
use crate::randomx_memory::{PageKind, Policy, Scratchpad};
use aes::HardwareAes;
use emitter::{Emitter, PROGRAM_SIZE, TOTAL_SIZE};
use memory::Executable;
use std::{io, mem::MaybeUninit, sync::Arc};

/// Pure Rust scalar cache initializer; SIMD selections are unavailable on this target.
pub fn new_cache(
    key: &[u8],
    argon2: Option<u32>,
) -> Result<Cache, crate::randomx_core::cache::CacheError> {
    new_cache_with_policy(key, argon2, Policy::default())
}

pub fn new_cache_with_policy(
    key: &[u8],
    argon2: Option<u32>,
    policy: Policy,
) -> Result<Cache, crate::randomx_core::cache::CacheError> {
    argon2::cache(key, argon2, policy)
}

#[derive(Clone, Copy, Debug)]
pub struct VmOptions {
    pub hardware_aes: bool,
    pub huge_pages: bool,
    pub jit_huge_pages: bool,
    pub secure: bool,
    pub prefetch: u8,
}
impl Default for VmOptions {
    fn default() -> Self {
        use std::sync::atomic::Ordering;
        Self {
            hardware_aes: true,
            huge_pages: false,
            secure: false,
            jit_huge_pages: crate::randomx_memory::JIT_HUGE.load(Ordering::Relaxed),
            prefetch: crate::randomx_memory::PREFETCH.load(Ordering::Relaxed),
        }
    }
}

pub enum Source {
    Light(Arc<Cache>),
    Full(Arc<Dataset>),
}
#[repr(C, align(64))]
#[derive(Default)]
struct Registers {
    words: [u64; 32],
}
impl Registers {
    fn bytes(&self) -> [u8; 256] {
        let mut out = [0; 256];
        for (chunk, word) in out.as_chunks_mut::<8>().0.iter_mut().zip(self.words) {
            *chunk = word.to_le_bytes();
        }
        out
    }
}
#[repr(C)]
struct Memory {
    mx: u32,
    ma: u32,
    pointer: *const u8,
}
struct RoundingGuard(u64, u64);
impl RoundingGuard {
    fn new() -> Self {
        let control;
        let status;
        // SAFETY: ARM64 FP control/status registers are accessible in userspace.
        unsafe {
            core::arch::asm!("mrs {c}, fpcr", "mrs {s}, fpsr", "msr fpcr, xzr", "msr fpsr, xzr", c=out(reg) control,s=out(reg) status, options(nostack,preserves_flags));
        }
        Self(control, status)
    }
}
impl Drop for RoundingGuard {
    fn drop(&mut self) {
        // SAFETY: restore exactly the caller's saved control and exception state.
        unsafe {
            core::arch::asm!("msr fpcr, {c}", "msr fpsr, {s}", c=in(reg) self.0,s=in(reg) self.1, options(nostack,preserves_flags));
        }
    }
}

pub struct Vm {
    source: Source,
    scratch: Scratchpad,
    code: Executable,
    emitter: Emitter,
    hardware: Option<HardwareAes>,
    software: Aes,
    pending: Option<[u8; 64]>,
    options: VmOptions,
}
impl Vm {
    pub fn new(source: Source, hardware_aes: bool) -> io::Result<Self> {
        Self::with_options(
            source,
            VmOptions {
                hardware_aes,
                ..VmOptions::default()
            },
        )
    }
    pub fn with_options(source: Source, options: VmOptions) -> io::Result<Self> {
        if options.prefetch != 1 {
            return Err(io::Error::other(
                "custom scratchpad prefetch modes require x86-64",
            ));
        }
        let hardware_aes = options.hardware_aes;
        let light = matches!(source, Source::Light(_));
        let mut emitter = Emitter::new(light);
        emitter.prefetch_mode = options.prefetch;
        let mut code = Executable::with_options(
            if light { TOTAL_SIZE } else { PROGRAM_SIZE },
            options.jit_huge_pages,
            options.secure,
        )?;
        if let Source::Light(cache) = &source {
            emitter.superscalar(cache);
            code.publish(&emitter.code)?;
        }
        Ok(Self {
            source,
            scratch: Scratchpad::new(options.huge_pages)?,
            code,
            emitter,
            hardware: if hardware_aes {
                HardwareAes::new()
            } else {
                None
            },
            software: Aes::default(),
            pending: None,
            options,
        })
    }
    pub fn uses_huge_pages(&self) -> bool {
        self.scratch.kind() == PageKind::Huge2M
    }
    pub fn uses_huge_jit_pages(&self) -> bool {
        self.code.huge()
    }
    pub fn set_source(&mut self, source: Source) -> io::Result<()> {
        if self.pending.is_some() {
            return Err(io::Error::other(
                "cannot replace data while a hash is pending",
            ));
        }
        if std::mem::discriminant(&source) != std::mem::discriminant(&self.source) {
            return Err(io::Error::other(
                "replacement must preserve full/light mode",
            ));
        }
        if let Source::Light(cache) = &source {
            let mut emitter = Emitter::new(true);
            emitter.prefetch_mode = self.options.prefetch;
            emitter.superscalar(cache);
            let mut code = Executable::with_options(
                TOTAL_SIZE,
                self.options.jit_huge_pages,
                self.options.secure,
            )?;
            code.publish(&emitter.code)?;
            self.emitter = emitter;
            self.code = code;
        }
        self.source = source;
        Ok(())
    }
    pub fn first(&mut self, input: &[u8]) {
        assert!(self.pending.is_none());
        let mut seed = blake2b::hash_512(input);
        match &self.hardware {
            Some(aes) => aes.fill(&mut seed, self.scratch.bytes_mut()),
            None => self.software.fill(&mut seed, self.scratch.bytes_mut()),
        }
        self.pending = Some(seed);
    }
    pub fn hash(&mut self, input: &[u8], v2: bool) -> io::Result<[u8; 32]> {
        self.first(input);
        self.last(v2)
    }
    pub fn next(&mut self, input: &[u8], v2: bool) -> io::Result<[u8; 32]> {
        let seed = blake2b::hash_512(input);
        self.finish(v2, Some(seed))
    }
    pub fn last(&mut self, v2: bool) -> io::Result<[u8; 32]> {
        self.finish(v2, None)
    }
    fn finish(&mut self, v2: bool, mut next: Option<[u8; 64]>) -> io::Result<[u8; 32]> {
        let mut seed = self.pending.take().expect("hash already pending");
        let _rounding = RoundingGuard::new();
        for _ in 0..7 {
            let registers = self.run(&seed, v2)?;
            seed = blake2b::hash_512(&registers.bytes());
        }
        let mut registers = self.run(&seed, v2)?;
        let hash = if let Some(aes) = &self.hardware {
            aes.finish(self.scratch.bytes_mut(), next.as_mut())
        } else {
            self.software
                .finish(self.scratch.bytes_mut(), next.as_mut())
        };
        for (word, bytes) in registers.words[24..]
            .iter_mut()
            .zip(hash.as_chunks::<8>().0)
        {
            *word = u64::from_le_bytes(*bytes);
        }
        let mut out = [0; 32];
        blake2b::hash(&registers.bytes(), &mut out).unwrap();
        self.pending = next;
        Ok(out)
    }
    fn run(&mut self, seed: &[u8; 64], v2: bool) -> io::Result<Registers> {
        let mut program = [0u8; 128 + 384 * 8];
        let size = 128 + if v2 { 384 } else { 256 } * 8;
        match &self.hardware {
            Some(aes) => aes.program(seed, &mut program[..size]),
            None => self.software.program(seed, &mut program[..size]),
        }
        let entropy = |i: usize| u64::from_le_bytes(program[i * 8..i * 8 + 8].try_into().unwrap());
        let mut registers = Registers::default();
        for i in 0..8 {
            let bits = entropy(i);
            registers.words[24 + i] = (((bits >> 59) + 1023) << 52) | (bits & ((1u64 << 52) - 1));
        }
        let read = std::array::from_fn(|i| 2 * i + ((entropy(12) >> i) & 1) as usize);
        let offset = (entropy(13) % 524288) * 64;
        let masks = [entropy(14), entropy(15)]
            .map(|bits| (bits & ((1 << 22) - 1)) | ((0x300 | ((bits >> 60) << 4)) << 52));
        registers.words[8..10].copy_from_slice(&masks);
        let light = matches!(self.source, Source::Light(_));
        self.emitter.compile(
            &program[128..size],
            emitter::ProgramConfig {
                read,
                offset: offset as u32,
                v2,
                light,
                hardware: self.hardware.is_some(),
            },
        );
        self.code
            .publish(&self.emitter.code[..templates::program().len()])?;
        let pointer = match &self.source {
            Source::Light(cache) => {
                let words = cache.words();
                if words.len() != crate::randomx_core::cache::CACHE_SIZE / 8 {
                    return Err(io::Error::other("invalid cache storage"));
                }
                words.as_ptr().cast()
            }
            Source::Full(data) => {
                // SAFETY: the extra dataset region is 32 MiB minus 64 bytes; offset
                // is 0..32 MiB in 64-byte steps. Generated addressing masks the
                // remaining offset to the first 2 GiB minus 64 bytes.
                let words = data.words();
                if words.len() != crate::randomx_core::cache::DATASET_ITEMS * 8 {
                    return Err(io::Error::other("invalid dataset storage"));
                }
                // SAFETY: validated the entire dataset length; offset is within the extra region.
                unsafe { words.as_ptr().cast::<u8>().add(offset as usize) }
            }
        };
        let memory = Memory {
            mx: entropy(10) as u32,
            ma: entropy(8) as u32 & 2147483584,
            pointer,
        };
        // SAFETY: only this verified emitter can populate the RX mapping. It
        // follows the AAPCS64 ABI, preserves callee-saved registers, bounds
        // all scratch/dataset/cache addresses, and writes only to the aligned
        // register file and scratchpad exclusively borrowed for this call.
        // Source owns the immutable data and generated light programs together.
        unsafe {
            let function: unsafe extern "C" fn(*mut Registers, *const Memory, *mut u8, u64) =
                std::mem::transmute(self.code.entry());
            function(
                &mut registers,
                &memory,
                self.scratch.bytes_mut().as_mut_ptr(),
                2048,
            );
        }
        Ok(registers)
    }
}

/// Immutable generated initializer; workers may share it with disjoint outputs.
pub struct Initializer<'a> {
    cache: &'a Cache,
    code: Executable,
}
impl<'a> Initializer<'a> {
    pub fn new(cache: &'a Cache) -> io::Result<Self> {
        let mut emitter = Emitter::new(true);
        emitter.dataset_initializer(cache);
        let mut code = Executable::new(TOTAL_SIZE)?;
        code.publish(&emitter.code)?;
        Ok(Self { cache, code })
    }
    /// Allocate OS-zeroed storage, then let the JIT write every item exactly
    /// once. The shared worker controller provides cancellation and joins all
    /// workers before any initialized dataset can be published.
    pub fn build_dataset(
        &self,
        threads: usize,
        stop: &std::sync::atomic::AtomicBool,
        setup: impl Fn(usize) -> Result<(), crate::randomx_core::cache::CacheError> + Sync,
    ) -> Result<Dataset, crate::randomx_core::cache::CacheError> {
        self.build_dataset_with_policy(threads, stop, setup, Policy::default())
    }
    pub fn build_dataset_with_policy(
        &self,
        threads: usize,
        stop: &std::sync::atomic::AtomicBool,
        setup: impl Fn(usize) -> Result<(), crate::randomx_core::cache::CacheError> + Sync,
        policy: Policy,
    ) -> Result<Dataset, crate::randomx_core::cache::CacheError> {
        use crate::randomx_core::cache::{CacheError, DATASET_ITEMS};
        if threads == 0 {
            return Err(CacheError::InvalidRange);
        }
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(CacheError::Interrupted);
        }
        let mut items = crate::randomx_memory::Mapped::<[u64; 8]>::new(DATASET_ITEMS, policy)
            .map_err(CacheError::Backend)?;
        crate::randomx_core::vm::initialize_dataset(
            &|start, output| self.fill(start, output).map_err(CacheError::Backend),
            items.as_mut(),
            threads,
            stop,
            &setup,
        )?;
        Dataset::from_memory(items.into_memory())
    }
    pub fn fill(&self, start: usize, output: &mut [[u64; 8]]) -> io::Result<()> {
        // SAFETY: valid mutable u64 arrays are also valid MaybeUninit arrays;
        // their layout is identical, and the callee initializes every item.
        let output = unsafe {
            std::slice::from_raw_parts_mut(
                output.as_mut_ptr().cast::<MaybeUninit<[u64; 8]>>(),
                output.len(),
            )
        };
        self.fill_uninitialized(start, output)
    }
    fn fill_uninitialized(
        &self,
        start: usize,
        output: &mut [MaybeUninit<[u64; 8]>],
    ) -> io::Result<()> {
        let end = start
            .checked_add(output.len())
            .filter(|&n| n <= crate::randomx_core::cache::DATASET_ITEMS)
            .ok_or_else(|| io::Error::other("dataset range"))?;
        if output.is_empty() {
            return Ok(());
        }
        let words = self.cache.words();
        if words.len() != crate::randomx_core::cache::CACHE_SIZE / 8 {
            return Err(io::Error::other("invalid cache storage"));
        }
        let pointer = words.as_ptr();
        // SAFETY: generated initializer follows AAPCS64 ABI and writes all
        // eight words of exactly end-start items, without reading prior output.
        // Exclusive output has that capacity; cache/code are live and immutable.
        unsafe {
            let function: unsafe extern "C" fn(*const *const u64, *mut u64, u64, u64) =
                std::mem::transmute(self.code.entry().add(templates::init_dataset_aarch64()));
            function(
                &pointer,
                output.as_mut_ptr().cast(),
                start as u64,
                end as u64,
            );
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../port_tests.rs"]
mod tests;
