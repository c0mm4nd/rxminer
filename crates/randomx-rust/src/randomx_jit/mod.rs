// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2020, tevador <tevador@gmail.com>
// Copyright (c) 2019-2021, SChernykh <https://github.com/SChernykh>
// Copyright (c) 2019-2021, XMRig <https://github.com/xmrig>
// Copyright (c) 2026 RxMiner contributors
//! Rust-owned x86-64 JIT, AES-NI and dataset initializer. The portable core
//! remains an independent, unsafe-free reference. No native RandomX FFI.
pub(crate) mod aes;
pub(crate) mod argon2;
mod argon2_ssse3;
mod memory;
mod templates;
mod x86_64;
#[cfg(test)]
use crate::randomx_core::aes::Aes;
use crate::randomx_core::{blake2b, cache::Cache, vm::Dataset};
use crate::randomx_memory::{PageKind, Policy, Scratchpad};
use aes::{HardwareAes, SoftwareAes};
use memory::Executable;
use std::{io, mem::MaybeUninit, sync::Arc};
use x86_64::{Emitter, PROGRAM_SIZE};

/// Runtime-selected pure Rust cache initializer; 0 forces scalar, 64 AVX2.
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
struct RoundingGuard(u32);
impl RoundingGuard {
    fn new() -> Self {
        let mut old = 0u32;
        let initial = 0x9fc0u32;
        // SAFETY: stack pointers address complete u32 control words. SSE2 and
        // MXCSR exist on every x86-64 CPU; exceptions are masked in initial.
        unsafe {
            std::arch::asm!("stmxcsr [{old}]","ldmxcsr [{initial}]",old=in(reg)&mut old,initial=in(reg)&initial,options(nostack,preserves_flags));
        }
        Self(old)
    }
}
impl Drop for RoundingGuard {
    fn drop(&mut self) {
        // SAFETY: restore the exact previously saved valid control word.
        unsafe {
            std::arch::asm!("ldmxcsr [{}]",in(reg)&self.0,options(nostack,preserves_flags));
        }
    }
}

// Only one backend is active. Keeping the choices in an enum avoids storing
// and initializing an unused second copy of the fixed RandomX AES constants.
enum VmAes {
    Hardware(HardwareAes),
    Software(SoftwareAes),
}
impl VmAes {
    fn new(hardware: bool) -> Self {
        if hardware {
            if let Some(aes) = HardwareAes::new() {
                return Self::Hardware(aes);
            }
        }
        Self::Software(SoftwareAes::default())
    }
    fn is_hardware(&self) -> bool {
        matches!(self, Self::Hardware(_))
    }
    fn fill(&self, state: &mut [u8; 64], output: &mut [u8]) {
        match self {
            Self::Hardware(aes) => aes.fill(state, output),
            Self::Software(aes) => aes.fill(state, output),
        }
    }
    fn program<'a>(
        &self,
        seed: &[u8; 64],
        storage: &'a mut [MaybeUninit<u8>],
        size: usize,
    ) -> &'a mut [u8] {
        match self {
            Self::Hardware(aes) => aes.program_uninit(seed, &mut storage[..size]),
            Self::Software(aes) => {
                // Preserve the measured software-generator preclear policy.
                storage.fill(MaybeUninit::new(0));
                aes.program_uninit(seed, &mut storage[..size])
            }
        }
    }
    fn finish(&self, input: &mut [u8], next: Option<&mut [u8; 64]>, prefetch: u8) -> [u8; 64] {
        match self {
            // The AVX/no-prefetch finisher regresses complete hashes with NTA
            // scratchpad prefetch (p2). Keep that mode's preceding finish path.
            Self::Hardware(aes) if prefetch == 2 => aes.finish_prefetched(input, next),
            Self::Hardware(aes) => aes.finish(input, next),
            Self::Software(aes) => aes.finish(input, next),
        }
    }
}

pub struct Vm {
    source: Source,
    scratch: Scratchpad,
    code: Executable,
    emitter: Emitter,
    aes: VmAes,
    pending: Option<[u8; 64]>,
    options: VmOptions,
    program_published: bool,
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
        if options.prefetch > 3 {
            return Err(io::Error::other("prefetch must be 0..3"));
        }
        let hardware_aes = options.hardware_aes;
        let light = matches!(source, Source::Light(_));
        let mut emitter = Emitter::new(light);
        emitter.prefetch_mode = options.prefetch;
        // Helper lengths depend on the cache key. Keep the fixed VM offsets,
        // but allocate/publish only the generated prefix, rounded to OS pages.
        let end = match &source {
            Source::Light(cache) => emitter.superscalar(cache),
            Source::Full(_) => PROGRAM_SIZE,
        };
        let mut code = Executable::with_options(
            end.next_multiple_of(4096),
            options.jit_huge_pages,
            options.secure,
        )?;
        if light {
            code.publish(&emitter.code[..end])?;
            emitter.discard_superscalar_copy();
        }
        Ok(Self {
            source,
            scratch: Scratchpad::new(options.huge_pages)?,
            code,
            emitter,
            aes: VmAes::new(hardware_aes),
            pending: None,
            options,
            program_published: false,
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
            let end = emitter.superscalar(cache);
            let mut code = Executable::with_options(
                end.next_multiple_of(4096),
                self.options.jit_huge_pages,
                self.options.secure,
            )?;
            code.publish(&emitter.code[..end])?;
            emitter.discard_superscalar_copy();
            self.emitter = emitter;
            self.code = code;
            self.program_published = false;
        }
        self.source = source;
        Ok(())
    }
    pub fn first(&mut self, input: &[u8]) {
        assert!(self.pending.is_none());
        let mut seed = blake2b::hash_512(input);
        self.aes.fill(&mut seed, self.scratch.bytes_mut());
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
        let hash = self.aes.finish(
            self.scratch.bytes_mut(),
            next.as_mut(),
            self.options.prefetch,
        );
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
        let mut storage = [MaybeUninit::<u8>::uninit(); 128 + 384 * 8];
        let size = 128 + if v2 { 384 } else { 256 } * 8;
        let program = self.aes.program(seed, &mut storage, size);
        // Establish the fixed entropy region once, so its sixteen constant
        // word offsets do not each require a check against the program length.
        let (entropy_bytes, instructions) = program
            .split_first_chunk::<128>()
            .expect("AES generator initialized the complete program");
        let entropy_words = entropy_bytes.as_chunks::<8>().0;
        let entropy = |i: usize| u64::from_le_bytes(entropy_words[i]);
        let mut registers = Registers::default();
        for i in 0..8 {
            let bits = entropy(i);
            registers.words[24 + i] = (((bits >> 59) + 1023) << 52) | (bits & ((1u64 << 52) - 1));
        }
        let read = std::array::from_fn(|i| 2 * i + ((entropy(12) >> i) & 1) as usize);
        let offset = (entropy(13) % 524288) * 64;
        let masks = [entropy(14), entropy(15)]
            .map(|bits| (bits & ((1 << 22) - 1)) | ((0x300 | ((bits >> 60) << 4)) << 52));
        let light = matches!(self.source, Source::Light(_));
        let end = self.emitter.compile(
            instructions,
            x86_64::ProgramConfig {
                read,
                masks,
                offset: offset as u32,
                v2,
                light,
                hardware: self.aes.is_hardware(),
            },
        );
        if self.program_published {
            // Only exponent masks, the program body and reciprocal slots can
            // change. Keep the already-published instruction prefix intact.
            self.code.publish_ranges(
                &self.emitter.code[..PROGRAM_SIZE],
                &[
                    Emitter::masks_offset()..end,
                    x86_64::RECIPROCALS..x86_64::ROUNDING,
                ],
            )?;
        } else {
            self.code.publish(&self.emitter.code[..PROGRAM_SIZE])?;
            self.program_published = true;
        }
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
        // follows the System V ABI, preserves callee-saved registers, bounds
        // all scratch/dataset/cache addresses, and writes only to the aligned
        // register file and scratchpad exclusively borrowed for this call.
        // Source owns the immutable data and generated light programs together.
        unsafe {
            let function: unsafe extern "sysv64" fn(*mut Registers, *const Memory, *mut u8, u64) =
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
        let end = emitter.dataset_initializer(cache);
        let mut code = Executable::new(end.next_multiple_of(4096))?;
        code.publish(&emitter.code[..end])?;
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
        // SAFETY: generated initializer follows System V ABI and writes all
        // eight words of exactly end-start items, without reading prior output.
        // Exclusive output has that capacity; cache/code are live and immutable.
        unsafe {
            let function: unsafe extern "sysv64" fn(*const *const u64, *mut u64, u64, u64) =
                std::mem::transmute(self.code.entry());
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
mod tests {
    use super::*;
    use crate::randomx_core::vm::SCRATCHPAD_SIZE;
    #[test]
    fn aes_matches_reference() {
        let Some(aes) = HardwareAes::new() else {
            return;
        };
        let software = Aes::default();
        let mut seed = [7; 64];
        let mut seed2 = seed;
        let mut output = vec![0; SCRATCHPAD_SIZE];
        let mut expected = output.clone();
        aes.fill(&mut seed, &mut output);
        software.fill(&mut seed2, &mut expected);
        assert_eq!(seed, seed2);
        assert_eq!(output, expected);
        assert_eq!(aes.finish(&mut output, None), software.hash(&expected));
        let hash = software.hash(&expected);
        assert_eq!(aes.finish(&mut output, Some(&mut seed)), hash);
        software.fill(&mut seed2, &mut expected);
        assert_eq!(seed, seed2);
        assert_eq!(output, expected);
        let mut a = [0; 3200];
        let mut b = a;
        aes.program(&seed, &mut a);
        software.program(&seed, &mut b);
        assert_eq!(a, b);
        assert_ne!(hash, [0; 64]);
    }
    #[test]
    #[ignore = "256 MiB cache, JIT and portable complete v1/v2 differential"]
    fn light_hashes_and_dataset_items() {
        let cache = Arc::new(Cache::new(b"test key 000").unwrap());
        let init = Initializer::new(&cache).unwrap();
        let mut items = [[0; 8]; 17];
        for start in [0, 12345, crate::randomx_core::cache::DATASET_ITEMS - 17] {
            init.fill(start, &mut items).unwrap();
            for (i, item) in items.iter().enumerate() {
                assert_eq!(*item, cache.dataset_item((start + i) as u64));
            }
        }
        let stopped = std::sync::atomic::AtomicBool::new(true);
        assert!(init.build_dataset(2, &stopped, |_| Ok(())).is_err());
        assert!(init
            .build_dataset(0, &std::sync::atomic::AtomicBool::new(false), |_| Ok(()))
            .is_err());
        assert!(init
            .build_dataset(2, &std::sync::atomic::AtomicBool::new(false), |_| Err(
                crate::randomx_core::cache::CacheError::InvalidRange
            ))
            .is_err());
        let mut reference = crate::randomx_core::vm::Vm::new().unwrap();
        for (hw, baseline) in [(false, false), (true, false), (false, true)] {
            let mut jit = Vm::new(Source::Light(cache.clone()), hw).unwrap();
            if baseline {
                jit.emitter.baseline_instructions();
            }
            for v2 in [false, true] {
                for input in [b"This is a test".as_slice(), b"Lorem ipsum dolor sit amet"] {
                    let expected = reference.hash(&*cache, input, v2);
                    assert_eq!(
                        jit.hash(input, v2).unwrap(),
                        expected,
                        "hw={hw} baseline={baseline} v2={v2}"
                    );
                }
            }
        }
    }
    #[test]
    #[ignore = "two 256 MiB caches and portable v1/v2 oracle"]
    fn light_source_replacement_regenerates_helpers() {
        let caches = [b"test key 000".as_slice(), b"another seed"]
            .map(|key| Arc::new(new_cache(key, None).unwrap()));
        let mapping_lengths = caches.each_ref().map(|cache| {
            let end = Emitter::new(true).superscalar(cache);
            let len = end.next_multiple_of(4096);
            assert!(len < x86_64::TOTAL_SIZE);
            eprintln!("helper bytes={end} mapping bytes={len}");
            let init_end = Emitter::new(true).dataset_initializer(cache);
            let init_len = init_end.next_multiple_of(4096);
            assert!(init_len < len);
            eprintln!("initializer bytes={init_end} mapping bytes={init_len}");
            let initializer = Initializer::new(cache).unwrap();
            assert_eq!(
                initializer.code.mapped_len(),
                if initializer.code.huge() {
                    2 << 20
                } else {
                    init_len
                }
            );
            let mut items = [[0; 8]; 17];
            for start in [0, crate::randomx_core::cache::DATASET_ITEMS - items.len()] {
                initializer.fill(start, &mut items).unwrap();
                for (i, item) in items.iter().enumerate() {
                    assert_eq!(*item, cache.dataset_item((start + i) as u64));
                }
            }
            len
        });
        let input = b"source replacement after discarding helper staging bytes";
        let mut reference = crate::randomx_core::vm::Vm::new().unwrap();
        let expected = caches
            .each_ref()
            .map(|cache| [false, true].map(|v2| reference.hash(&**cache, input, v2)));
        assert_ne!(expected[0], expected[1]);
        for secure in [false, true] {
            for hardware_aes in [false, true] {
                for jit_huge_pages in [false, true] {
                    let mut vm = Vm::with_options(
                        Source::Light(caches[0].clone()),
                        VmOptions {
                            secure,
                            hardware_aes,
                            jit_huge_pages,
                            ..VmOptions::default()
                        },
                    )
                    .unwrap();
                    for (step, key) in [0, 1, 0].into_iter().enumerate() {
                        if step != 0 {
                            vm.set_source(Source::Light(caches[key].clone())).unwrap();
                        }
                        assert_eq!(
                            vm.code.mapped_len(),
                            if vm.code.huge() {
                                2 << 20
                            } else {
                                mapping_lengths[key]
                            }
                        );
                        for v2 in [false, true] {
                            assert_eq!(
                                vm.hash(input, v2).unwrap(),
                                expected[key][v2 as usize],
                                "key={key} options={:?} v2={v2}",
                                vm.options
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    #[ignore = "complete dataset, full/light pipeline, software/hardware AES, v1/v2 and MXCSR"]
    fn full_hashes_pipeline_and_mxcsr() {
        let cache = Arc::new(Cache::new(b"test key 000").unwrap());
        let init = Initializer::new(&cache).unwrap();
        let data = Arc::new(
            init.build_dataset(8, &std::sync::atomic::AtomicBool::new(false), |_| Ok(()))
                .unwrap(),
        );
        let inputs = [
            b"This is a test".as_slice(),
            b"Lorem ipsum dolor sit amet",
            b"different input",
        ];
        let versions = [false, true, false];
        let mut reference = crate::randomx_core::vm::Vm::new().unwrap();
        let expected: Vec<_> = inputs
            .iter()
            .zip(versions)
            .map(|(input, v2)| reference.hash(&*cache, input, v2))
            .collect();
        for full in [false, true] {
            for hardware in [false, true] {
                let source = if full {
                    Source::Full(data.clone())
                } else {
                    Source::Light(cache.clone())
                };
                let mut jit = Vm::new(source, hardware).unwrap();
                // Exercise a caller with nondefault rounding. Guard restores even
                // an unwinding test, and every hash must preserve that exact word.
                let _restore = RoundingGuard::new();
                let control = 0xdfc0u32;
                // SAFETY: valid MXCSR word in a complete aligned stack object.
                unsafe {
                    std::arch::asm!("ldmxcsr [{}]",in(reg)&control,options(nostack,preserves_flags));
                }
                jit.first(inputs[0]);
                for i in 0..inputs.len() {
                    let hash = if i + 1 < inputs.len() {
                        jit.next(inputs[i + 1], versions[i])
                    } else {
                        jit.last(versions[i])
                    }
                    .unwrap();
                    assert_eq!(
                        hash, expected[i],
                        "full={full} hardware={hardware} input={i}"
                    );
                    let mut after = 0u32;
                    // SAFETY: store current MXCSR to a complete stack u32.
                    unsafe {
                        std::arch::asm!("stmxcsr [{}]",in(reg)&mut after,options(nostack,preserves_flags));
                    }
                    assert_eq!(after, control);
                }
            }
        }
    }
}
