// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
//! Runtime-gated AES-NI versions of the reference core's AES constructions.
use crate::randomx_core::aes::Aes;
use std::{arch::x86_64::*, mem::MaybeUninit};
pub(crate) struct HardwareAes {
    keys: Aes,
}
impl HardwareAes {
    pub fn new() -> Option<Self> {
        std::is_x86_feature_detected!("aes").then(|| Self {
            keys: Aes::default(),
        })
    }
    pub fn fill(&self, state: &mut [u8; 64], output: &mut [u8]) {
        assert!(output.len().is_multiple_of(64));
        // SAFETY: construction checks AES-NI; all input/output lengths are checked.
        unsafe { fill(&self.keys, state, output) }
    }
    pub fn program(&self, state: &[u8; 64], output: &mut [u8]) {
        // SAFETY: initialized bytes are valid MaybeUninit<u8> values, with the
        // same layout. The exclusive borrow covers the complete output slice.
        let output =
            unsafe { std::slice::from_raw_parts_mut(output.as_mut_ptr().cast(), output.len()) };
        self.program_uninit(state, output);
    }
    /// Initialize every output byte, returning only the initialized region.
    pub fn program_uninit<'a>(
        &self,
        state: &[u8; 64],
        output: &'a mut [MaybeUninit<u8>],
    ) -> &'a mut [u8] {
        assert!(output.len().is_multiple_of(64));
        // SAFETY: construction checks AES-NI. Each complete 64-byte block
        // receives four full 16-byte stores; the generator never reads output.
        unsafe {
            program(&self.keys, state, output);
            std::slice::from_raw_parts_mut(output.as_mut_ptr().cast(), output.len())
        }
    }
    pub fn finish(&self, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
        assert!(input.len().is_multiple_of(64));
        // SAFETY: construction checks AES-NI; the AVX path also checks OS/CPU
        // support. Both paths accept unaligned complete lanes and states.
        unsafe {
            if std::is_x86_feature_detected!("avx") {
                finish_avx(&self.keys, input, next)
            } else {
                finish(&self.keys, input, next)
            }
        }
    }
    pub(super) fn finish_prefetched(
        &self,
        input: &mut [u8],
        next: Option<&mut [u8; 64]>,
    ) -> [u8; 64] {
        assert!(input.len().is_multiple_of(64));
        // SAFETY: construction checks AES-NI; chunks and states have complete lanes.
        unsafe { finish(&self.keys, input, next) }
    }
}
#[target_feature(enable = "aes")]
unsafe fn fill(keys: &Aes, state: &mut [u8; 64], output: &mut [u8]) {
    // SAFETY: every unaligned load/store addresses a complete 16-byte lane;
    // arrays/chunks enforce these bounds. AES is enabled for this function.
    unsafe {
        let mut s: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(state.as_ptr().add(i * 16).cast()));
        let k: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(keys.fill_keys[i].as_ptr().cast()));
        for b in output.as_chunks_mut::<64>().0 {
            s[0] = _mm_aesdec_si128(s[0], k[0]);
            s[1] = _mm_aesenc_si128(s[1], k[1]);
            s[2] = _mm_aesdec_si128(s[2], k[2]);
            s[3] = _mm_aesenc_si128(s[3], k[3]);
            for (i, &lane) in s.iter().enumerate() {
                _mm_storeu_si128(b.as_mut_ptr().add(i * 16).cast(), lane);
            }
        }
        for (i, &lane) in s.iter().enumerate() {
            _mm_storeu_si128(state.as_mut_ptr().add(i * 16).cast(), lane);
        }
    }
}
#[target_feature(enable = "aes")]
unsafe fn program(keys: &Aes, state: &[u8; 64], output: &mut [MaybeUninit<u8>]) {
    // SAFETY: complete lanes within checked arrays/chunks; AES is enabled.
    unsafe {
        let mut s: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(state.as_ptr().add(i * 16).cast()));
        let k: [__m128i; 8] =
            std::array::from_fn(|i| _mm_loadu_si128(keys.program_keys[i].as_ptr().cast()));
        for b in output.as_chunks_mut::<64>().0 {
            for j in 0..4 {
                s[0] = _mm_aesdec_si128(s[0], k[j]);
                s[1] = _mm_aesenc_si128(s[1], k[j]);
                s[2] = _mm_aesdec_si128(s[2], k[j + 4]);
                s[3] = _mm_aesenc_si128(s[3], k[j + 4]);
            }
            for (i, &lane) in s.iter().enumerate() {
                _mm_storeu_si128(b.as_mut_ptr().add(i * 16).cast(), lane);
            }
        }
    }
}
#[target_feature(enable = "aes")]
unsafe fn finish(keys: &Aes, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
    // SAFETY: caller supplies complete lanes and this function enables AES-NI.
    unsafe { finish_inner::<true>(keys, input, next) }
}
#[target_feature(enable = "aes,avx")]
unsafe fn finish_avx(keys: &Aes, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
    // SAFETY: caller checks AES/AVX and supplies complete lanes. AVX permits
    // folding unaligned loads into the 128-bit AES instructions. Sequential
    // scratch traversal uses hardware prefetching on this path.
    unsafe { finish_inner::<false>(keys, input, next) }
}
#[inline(always)]
unsafe fn finish_inner<const PREFETCH: bool>(
    keys: &Aes,
    input: &mut [u8],
    next: Option<&mut [u8; 64]>,
) -> [u8; 64] {
    // SAFETY: complete 16-byte lanes within checked arrays/chunks. Each scratch
    // chunk is loaded before refill overwrites it. AES is enabled.
    unsafe {
        let mut hash: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(keys.hash_state[i].as_ptr().cast()));
        let mut fill = next.as_ref().map(|n| {
            std::array::from_fn::<_, 4, _>(|i| _mm_loadu_si128(n.as_ptr().add(i * 16).cast()))
        });
        let k: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(keys.fill_keys[i].as_ptr().cast()));
        let length = input.len();
        let pointer = input.as_ptr();
        let (pairs, tail) = input.as_chunks_mut::<128>();
        for (index, pair) in pairs.iter_mut().enumerate() {
            for b in pair.as_chunks::<64>().0 {
                hash[0] = _mm_aesenc_si128(hash[0], _mm_loadu_si128(b.as_ptr().cast()));
                hash[1] = _mm_aesdec_si128(hash[1], _mm_loadu_si128(b.as_ptr().add(16).cast()));
                hash[2] = _mm_aesenc_si128(hash[2], _mm_loadu_si128(b.as_ptr().add(32).cast()));
                hash[3] = _mm_aesdec_si128(hash[3], _mm_loadu_si128(b.as_ptr().add(48).cast()));
            }
            if let Some(s) = &mut fill {
                for b in pair.as_chunks_mut::<64>().0 {
                    s[0] = _mm_aesdec_si128(s[0], k[0]);
                    s[1] = _mm_aesenc_si128(s[1], k[1]);
                    s[2] = _mm_aesdec_si128(s[2], k[2]);
                    s[3] = _mm_aesenc_si128(s[3], k[3]);
                    for (i, &lane) in s.iter().enumerate() {
                        _mm_storeu_si128(b.as_mut_ptr().add(i * 16).cast(), lane);
                    }
                }
            }
            if PREFETCH {
                let ahead = if index * 128 + 4096 + 64 < length {
                    index * 128 + 4096
                } else {
                    0
                };
                _mm_prefetch(pointer.add(ahead).cast(), _MM_HINT_T0);
                _mm_prefetch(pointer.add(ahead + 64).cast(), _MM_HINT_T0);
            }
        }
        for b in tail.as_chunks_mut::<64>().0 {
            let x: [__m128i; 4] =
                std::array::from_fn(|i| _mm_loadu_si128(b.as_ptr().add(i * 16).cast()));
            hash[0] = _mm_aesenc_si128(hash[0], x[0]);
            hash[1] = _mm_aesdec_si128(hash[1], x[1]);
            hash[2] = _mm_aesenc_si128(hash[2], x[2]);
            hash[3] = _mm_aesdec_si128(hash[3], x[3]);
            if let Some(s) = &mut fill {
                s[0] = _mm_aesdec_si128(s[0], k[0]);
                s[1] = _mm_aesenc_si128(s[1], k[1]);
                s[2] = _mm_aesdec_si128(s[2], k[2]);
                s[3] = _mm_aesenc_si128(s[3], k[3]);
                for (i, &lane) in s.iter().enumerate() {
                    _mm_storeu_si128(b.as_mut_ptr().add(i * 16).cast(), lane);
                }
            }
        }
        for k in &keys.extra {
            let k = _mm_loadu_si128(k.as_ptr().cast());
            hash[0] = _mm_aesenc_si128(hash[0], k);
            hash[1] = _mm_aesdec_si128(hash[1], k);
            hash[2] = _mm_aesenc_si128(hash[2], k);
            hash[3] = _mm_aesdec_si128(hash[3], k);
        }
        if let (Some(next), Some(s)) = (next, fill) {
            for (i, &lane) in s.iter().enumerate() {
                _mm_storeu_si128(next.as_mut_ptr().add(i * 16).cast(), lane);
            }
        }
        let mut out = [0; 64];
        for (i, &lane) in hash.iter().enumerate() {
            _mm_storeu_si128(out.as_mut_ptr().add(i * 16).cast(), lane);
        }
        out
    }
}

pub(super) use crate::randomx_core::aes::{DECODE, ENCODE};

impl crate::randomx_core::aes::Backend for HardwareAes {
    fn fill(&self, state: &mut [u8; 64], output: &mut [u8]) {
        self.fill(state, output);
    }
    fn program(&self, state: &[u8; 64], output: &mut [u8]) {
        self.program(state, output);
    }
    fn finish(&self, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
        self.finish(input, next)
    }
    fn mix(&self, f: &mut [u64; 8], e: &[u64; 8]) {
        // SAFETY: construction detects AES; arrays hold exactly four full lanes.
        unsafe {
            mix(f, e);
        }
    }
}
#[target_feature(enable = "aes")]
pub(crate) unsafe fn mix(f: &mut [u64; 8], e: &[u64; 8]) {
    // SAFETY: complete 16-byte lanes within the two arrays; AES is enabled.
    unsafe {
        let mut lanes: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(f.as_ptr().add(2 * i).cast()));
        for i in 0..4 {
            let key = _mm_loadu_si128(e.as_ptr().add(2 * i).cast());
            lanes[0] = _mm_aesenc_si128(lanes[0], key);
            lanes[1] = _mm_aesdec_si128(lanes[1], key);
            lanes[2] = _mm_aesenc_si128(lanes[2], key);
            lanes[3] = _mm_aesdec_si128(lanes[3], key);
        }
        for (i, lane) in lanes.into_iter().enumerate() {
            _mm_storeu_si128(f.as_mut_ptr().add(2 * i).cast(), lane);
        }
    }
}

#[derive(Default)]
pub(crate) struct SoftwareAes {
    keys: Aes,
}
impl SoftwareAes {
    pub fn fill(&self, state: &mut [u8; 64], out: &mut [u8]) {
        assert!(out.len().is_multiple_of(64));
        // SAFETY: complete byte lanes; SSE2 is part of x86-64, no AES required.
        unsafe { soft_fill(&self.keys, state, out) }
    }
    pub fn program(&self, state: &[u8; 64], out: &mut [u8]) {
        // SAFETY: initialized bytes are valid MaybeUninit<u8> values, with the
        // same layout. The exclusive borrow covers the complete output slice.
        let output = unsafe { std::slice::from_raw_parts_mut(out.as_mut_ptr().cast(), out.len()) };
        self.program_uninit(state, output);
    }
    /// Initialize every output byte, returning only the initialized region.
    pub fn program_uninit<'a>(
        &self,
        state: &[u8; 64],
        output: &'a mut [MaybeUninit<u8>],
    ) -> &'a mut [u8] {
        assert!(output.len().is_multiple_of(64));
        // SAFETY: SSE2 is baseline on x86-64. Each complete 64-byte block
        // receives four full 16-byte stores; the generator never reads output.
        unsafe {
            soft_program(&self.keys, state, output);
            std::slice::from_raw_parts_mut(output.as_mut_ptr().cast(), output.len())
        }
    }
    pub fn finish(&self, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
        assert!(input.len().is_multiple_of(64));
        // SAFETY: checked complete byte lanes and states, SSE2 baseline.
        unsafe { soft_finish(&self.keys, input, next) }
    }
}
impl crate::randomx_core::aes::Backend for SoftwareAes {
    fn fill(&self, state: &mut [u8; 64], out: &mut [u8]) {
        self.fill(state, out)
    }
    fn program(&self, state: &[u8; 64], out: &mut [u8]) {
        self.program(state, out)
    }
    fn finish(&self, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
        self.finish(input, next)
    }
    fn mix(&self, f: &mut [u64; 8], e: &[u64; 8]) {
        // SAFETY: complete arrays of four lanes; baseline SSE2 only.
        unsafe { soft_mix(f, e) }
    }
}
#[inline(always)]
unsafe fn soft_round<const DECRYPT: bool>(mut state: __m128i, key: __m128i) -> __m128i {
    let table = if DECRYPT { &DECODE } else { &ENCODE };
    // SAFETY: SSE2 baseline. Byte indices address immutable complete tables.
    // Fixed register scheduling keeps temporary lookup indices off the stack.
    unsafe {
        if DECRYPT {
            std::arch::asm!(
"movq rdx, {state}",
"movzx ecx, dl",
"mov {o0:e}, dword ptr [{table}+rcx*4+0]",
"movzx ecx, dh",
"mov {o1:e}, dword ptr [{table}+rcx*4+1024]",
"shr rdx, 16",
"movzx ecx, dl",
"mov {o2:e}, dword ptr [{table}+rcx*4+2048]",
"movzx ecx, dh",
"mov {o3:e}, dword ptr [{table}+rcx*4+3072]",
"shr rdx, 16",
"movzx ecx, dl",
"xor {o1:e}, dword ptr [{table}+rcx*4+0]",
"movzx ecx, dh",
"xor {o2:e}, dword ptr [{table}+rcx*4+1024]",
"shr rdx, 16",
"movzx ecx, dl",
"xor {o3:e}, dword ptr [{table}+rcx*4+2048]",
"movzx ecx, dh",
"xor {o0:e}, dword ptr [{table}+rcx*4+3072]",
"psrldq {state}, 8",
"movq rdx, {state}",
"movzx ecx, dl",
"xor {o2:e}, dword ptr [{table}+rcx*4+0]",
"movzx ecx, dh",
"xor {o3:e}, dword ptr [{table}+rcx*4+1024]",
"shr rdx, 16",
"movzx ecx, dl",
"xor {o0:e}, dword ptr [{table}+rcx*4+2048]",
"movzx ecx, dh",
"xor {o1:e}, dword ptr [{table}+rcx*4+3072]",
"shr rdx, 16",
"movzx ecx, dl",
"xor {o3:e}, dword ptr [{table}+rcx*4+0]",
"movzx ecx, dh",
"xor {o0:e}, dword ptr [{table}+rcx*4+1024]",
"shr rdx, 16",
"movzx ecx, dl",
"xor {o1:e}, dword ptr [{table}+rcx*4+2048]",
"movzx ecx, dh",
"xor {o2:e}, dword ptr [{table}+rcx*4+3072]",
"movd {state}, {o0:e}",
"movd {t1}, {o1:e}",
"punpckldq {state}, {t1}",
"movd {t1}, {o2:e}",
"movd {t2}, {o3:e}",
"punpckldq {t1}, {t2}",
"punpcklqdq {state}, {t1}",
"pxor {state}, {key}",
 state=inout(xmm_reg)state,key=in(xmm_reg)key,table=in(reg)table.as_ptr(),
 o0=out(reg)_,o1=out(reg)_,o2=out(reg)_,o3=out(reg)_,t1=out(xmm_reg)_,t2=out(xmm_reg)_,
 out("rcx")_,out("rdx")_,options(pure,readonly,nostack));
        } else {
            std::arch::asm!(
"movq rdx, {state}",
"movzx ecx, dl",
"mov {o0:e}, dword ptr [{table}+rcx*4+0]",
"movzx ecx, dh",
"mov {o3:e}, dword ptr [{table}+rcx*4+1024]",
"shr rdx, 16",
"movzx ecx, dl",
"mov {o2:e}, dword ptr [{table}+rcx*4+2048]",
"movzx ecx, dh",
"mov {o1:e}, dword ptr [{table}+rcx*4+3072]",
"shr rdx, 16",
"movzx ecx, dl",
"xor {o1:e}, dword ptr [{table}+rcx*4+0]",
"movzx ecx, dh",
"xor {o0:e}, dword ptr [{table}+rcx*4+1024]",
"shr rdx, 16",
"movzx ecx, dl",
"xor {o3:e}, dword ptr [{table}+rcx*4+2048]",
"movzx ecx, dh",
"xor {o2:e}, dword ptr [{table}+rcx*4+3072]",
"psrldq {state}, 8",
"movq rdx, {state}",
"movzx ecx, dl",
"xor {o2:e}, dword ptr [{table}+rcx*4+0]",
"movzx ecx, dh",
"xor {o1:e}, dword ptr [{table}+rcx*4+1024]",
"shr rdx, 16",
"movzx ecx, dl",
"xor {o0:e}, dword ptr [{table}+rcx*4+2048]",
"movzx ecx, dh",
"xor {o3:e}, dword ptr [{table}+rcx*4+3072]",
"shr rdx, 16",
"movzx ecx, dl",
"xor {o3:e}, dword ptr [{table}+rcx*4+0]",
"movzx ecx, dh",
"xor {o2:e}, dword ptr [{table}+rcx*4+1024]",
"shr rdx, 16",
"movzx ecx, dl",
"xor {o1:e}, dword ptr [{table}+rcx*4+2048]",
"movzx ecx, dh",
"xor {o0:e}, dword ptr [{table}+rcx*4+3072]",
"movd {state}, {o0:e}",
"movd {t1}, {o1:e}",
"punpckldq {state}, {t1}",
"movd {t1}, {o2:e}",
"movd {t2}, {o3:e}",
"punpckldq {t1}, {t2}",
"punpcklqdq {state}, {t1}",
"pxor {state}, {key}",
 state=inout(xmm_reg)state,key=in(xmm_reg)key,table=in(reg)table.as_ptr(),
 o0=out(reg)_,o1=out(reg)_,o2=out(reg)_,o3=out(reg)_,t1=out(xmm_reg)_,t2=out(xmm_reg)_,
 out("rcx")_,out("rdx")_,options(pure,readonly,nostack));
        }
    }
    state
}

unsafe fn soft_fill(keys: &Aes, state: &mut [u8; 64], output: &mut [u8]) {
    // SAFETY: every unaligned load/store addresses a complete 16-byte lane;
    // arrays/chunks enforce these bounds. SSE2 is baseline for this function.
    unsafe {
        let mut s: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(state.as_ptr().add(i * 16).cast()));
        let k: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(keys.fill_keys[i].as_ptr().cast()));
        for b in output.as_chunks_mut::<64>().0 {
            s[0] = soft_round::<true>(s[0], k[0]);
            s[1] = soft_round::<false>(s[1], k[1]);
            s[2] = soft_round::<true>(s[2], k[2]);
            s[3] = soft_round::<false>(s[3], k[3]);
            for (i, &lane) in s.iter().enumerate() {
                _mm_storeu_si128(b.as_mut_ptr().add(i * 16).cast(), lane);
            }
        }
        for (i, &lane) in s.iter().enumerate() {
            _mm_storeu_si128(state.as_mut_ptr().add(i * 16).cast(), lane);
        }
    }
}
unsafe fn soft_program(keys: &Aes, state: &[u8; 64], output: &mut [MaybeUninit<u8>]) {
    // SAFETY: complete lanes within checked arrays/chunks; SSE2 is baseline.
    unsafe {
        let mut s: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(state.as_ptr().add(i * 16).cast()));
        let k: [__m128i; 8] =
            std::array::from_fn(|i| _mm_loadu_si128(keys.program_keys[i].as_ptr().cast()));
        for b in output.as_chunks_mut::<64>().0 {
            for j in 0..4 {
                s[0] = soft_round::<true>(s[0], k[j]);
                s[1] = soft_round::<false>(s[1], k[j]);
                s[2] = soft_round::<true>(s[2], k[j + 4]);
                s[3] = soft_round::<false>(s[3], k[j + 4]);
            }
            for (i, &lane) in s.iter().enumerate() {
                _mm_storeu_si128(b.as_mut_ptr().add(i * 16).cast(), lane);
            }
        }
    }
}
unsafe fn soft_finish(keys: &Aes, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
    // SAFETY: complete 16-byte lanes within checked arrays/chunks. Each scratch
    // chunk is loaded before refill overwrites it. SSE2 is baseline.
    unsafe {
        let mut hash: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(keys.hash_state[i].as_ptr().cast()));
        let mut fill = next.as_ref().map(|n| {
            std::array::from_fn::<_, 4, _>(|i| _mm_loadu_si128(n.as_ptr().add(i * 16).cast()))
        });
        let k: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(keys.fill_keys[i].as_ptr().cast()));
        let pointer = input.as_ptr();
        let blocks = input.as_chunks_mut::<64>().0;
        // The prefix prefetches 4 KiB ahead; the final blocks wrap to the
        // beginning. Empty/short inputs use only the bounded tail path.
        let split = blocks.len().saturating_sub(64);
        let (head, tail) = blocks.split_at_mut(split);
        if !head.is_empty() {
            soft_finish_blocks(&mut hash, &mut fill, &k, head, pointer.add(4096));
        }
        soft_finish_blocks(&mut hash, &mut fill, &k, tail, pointer);
        for k in &keys.extra {
            let k = _mm_loadu_si128(k.as_ptr().cast());
            hash[0] = soft_round::<false>(hash[0], k);
            hash[1] = soft_round::<true>(hash[1], k);
            hash[2] = soft_round::<false>(hash[2], k);
            hash[3] = soft_round::<true>(hash[3], k);
        }
        if let (Some(next), Some(s)) = (next, fill) {
            for (i, &lane) in s.iter().enumerate() {
                _mm_storeu_si128(next.as_mut_ptr().add(i * 16).cast(), lane);
            }
        }
        let mut out = [0; 64];
        for (i, &lane) in hash.iter().enumerate() {
            _mm_storeu_si128(out.as_mut_ptr().add(i * 16).cast(), lane);
        }
        out
    }
}

#[inline(always)]
unsafe fn soft_finish_blocks(
    hash: &mut [__m128i; 4],
    fill: &mut Option<[__m128i; 4]>,
    k: &[__m128i; 4],
    blocks: &mut [[u8; 64]],
    prefetch: *const u8,
) {
    // SAFETY: SSE2 is baseline. The caller supplies complete blocks and a
    // prefetch range inside the original input. Each block is read before refill.
    unsafe {
        for (index, b) in blocks.iter_mut().enumerate() {
            _mm_prefetch(prefetch.add(index * 64).cast(), _MM_HINT_T0);
            let x: [__m128i; 4] =
                std::array::from_fn(|i| _mm_loadu_si128(b.as_ptr().add(i * 16).cast()));
            hash[0] = soft_round::<false>(hash[0], x[0]);
            hash[1] = soft_round::<true>(hash[1], x[1]);
            hash[2] = soft_round::<false>(hash[2], x[2]);
            hash[3] = soft_round::<true>(hash[3], x[3]);
            if let Some(s) = fill.as_mut() {
                s[0] = soft_round::<true>(s[0], k[0]);
                s[1] = soft_round::<false>(s[1], k[1]);
                s[2] = soft_round::<true>(s[2], k[2]);
                s[3] = soft_round::<false>(s[3], k[3]);
                for (i, &lane) in s.iter().enumerate() {
                    _mm_storeu_si128(b.as_mut_ptr().add(i * 16).cast(), lane);
                }
            }
        }
    }
}

unsafe fn soft_mix(f: &mut [u64; 8], e: &[u64; 8]) {
    // SAFETY: complete 16-byte lanes within the two arrays; SSE2 is baseline.
    unsafe {
        let mut lanes: [__m128i; 4] =
            std::array::from_fn(|i| _mm_loadu_si128(f.as_ptr().add(2 * i).cast()));
        for i in 0..4 {
            let key = _mm_loadu_si128(e.as_ptr().add(2 * i).cast());
            lanes[0] = soft_round::<false>(lanes[0], key);
            lanes[1] = soft_round::<true>(lanes[1], key);
            lanes[2] = soft_round::<false>(lanes[2], key);
            lanes[3] = soft_round::<true>(lanes[3], key);
        }
        for (i, lane) in lanes.into_iter().enumerate() {
            _mm_storeu_si128(f.as_mut_ptr().add(2 * i).cast(), lane);
        }
    }
}

#[cfg(test)]
mod software_tests {
    use super::*;
    use crate::randomx_core::aes::Backend;
    #[test]
    fn program_initializes_only_the_requested_region() {
        let software = SoftwareAes::default();
        let hardware = HardwareAes::new();
        let reference = Aes::default();
        for size in [0, 64, 2176, 3200] {
            for offset in 0..16 {
                let seed = std::array::from_fn(|i| (i * 17 + offset) as u8);
                let mut expected = vec![0; size];
                reference.program(&seed, &mut expected);
                for hw in [false, true] {
                    if hw && hardware.is_none() {
                        continue;
                    }
                    let mut storage = vec![MaybeUninit::uninit(); offset + size + 16];
                    for byte in storage[..offset].iter_mut() {
                        byte.write(0xa7);
                    }
                    for byte in storage[offset + size..].iter_mut() {
                        byte.write(0xa7);
                    }
                    let output = &mut storage[offset..offset + size];
                    let output = if hw {
                        hardware.as_ref().unwrap().program_uninit(&seed, output)
                    } else {
                        software.program_uninit(&seed, output)
                    };
                    assert_eq!(output, expected);
                    for byte in storage[..offset].iter().chain(&storage[offset + size..]) {
                        // SAFETY: both guard regions were explicitly initialized above.
                        assert_eq!(unsafe { byte.assume_init() }, 0xa7);
                    }
                }
            }
        }
    }
    #[test]
    fn finish_prefetch_boundaries_match_portable() {
        let software = SoftwareAes::default();
        let hardware = HardwareAes::new();
        let reference = Aes::default();
        for size in [0, 64, 128, 192, 4032, 4096, 4160, 4224, 4288, 2 << 20] {
            for offset in 0..16 {
                for refill in [false, true] {
                    let input: Vec<_> = (0..size).map(|i| (i * 13 + offset) as u8).collect();
                    let seed = [17; 64];
                    let mut expected = input.clone();
                    let mut expected_state = seed;
                    let hash =
                        reference.finish(&mut expected, refill.then_some(&mut expected_state));
                    for mode in 0..3 {
                        if mode != 0 && hardware.is_none() {
                            continue;
                        }
                        let mut guarded = vec![0xa7; offset + size + 16];
                        guarded[offset..offset + size].copy_from_slice(&input);
                        let mut state = seed;
                        let input = &mut guarded[offset..offset + size];
                        let next = refill.then_some(&mut state);
                        let actual = match mode {
                            0 => software.finish(input, next),
                            1 => hardware.as_ref().unwrap().finish(input, next),
                            // Exercise the prefetched/non-AVX path even when
                            // automatic dispatch on this host uses AVX.
                            _ => hardware.as_ref().unwrap().finish_prefetched(input, next),
                        };
                        assert_eq!(actual, hash);
                        assert_eq!(state, expected_state);
                        assert_eq!(&guarded[offset..offset + size], expected);
                        assert!(guarded[..offset]
                            .iter()
                            .chain(&guarded[offset + size..])
                            .all(|&byte| byte == 0xa7));
                    }
                }
            }
        }
    }

    #[test]
    fn sse2_software_matches_portable_constructions() {
        let a = SoftwareAes::default();
        let b = Aes::default();
        for size in [0, 64, 2176, 3200, 2 << 20] {
            let mut x = vec![0; size];
            let mut y = x.clone();
            let mut s = [17; 64];
            let mut t = s;
            a.fill(&mut s, &mut x);
            b.fill(&mut t, &mut y);
            assert_eq!(x, y);
            assert_eq!(s, t);
            a.program(&s, &mut x);
            b.program(&t, &mut y);
            assert_eq!(x, y);
            assert_eq!(
                a.finish(&mut x, Some(&mut s)),
                b.finish(&mut y, Some(&mut t))
            );
            assert_eq!(x, y);
            assert_eq!(s, t);
            assert_eq!(a.finish(&mut x, None), b.hash(&y));
        }
        let mut f = [0x1234567812345678; 8];
        let e = [0x8765432187654321; 8];
        let mut g = f;
        a.mix(&mut f, &e);
        b.mix(&mut g, &e);
        assert_eq!(f, g);
    }
}
