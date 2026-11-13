// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
// AVX2 BlaMka port from RandomX's Argon2 implementation. Original Argon2
// code by Dinu, Khovratovich, Aumasson and Neves was released under CC0.
use crate::randomx_core::cache::{Cache, CacheError, CACHE_SIZE};
use std::arch::x86_64::*;

pub(super) fn cache(
    key: &[u8],
    force: Option<u32>,
    policy: crate::randomx_memory::Policy,
) -> Result<Cache, CacheError> {
    if !available(force) {
        return Err(CacheError::Backend(std::io::Error::other(
            "requested Argon2 backend unavailable",
        )));
    }
    u32::try_from(key.len()).map_err(|_| CacheError::KeyTooLong)?;
    let storage = crate::randomx_memory::Mapped::<[u64; 128]>::new(CACHE_SIZE / 1024, policy)
        .map_err(CacheError::Backend)?
        .into_memory();
    let fill =
        if matches!(force, None | Some(64) | Some(96)) && std::is_x86_feature_detected!("avx2") {
            fill_avx2 as fn(&mut [[u64; 128]])
        } else if force != Some(0) && std::is_x86_feature_detected!("ssse3") {
            fill_ssse3
        } else {
            crate::randomx_core::cache::fill_memory
        };
    Cache::with_memory(key, storage, fill)
}
fn fill_avx2(memory: &mut [[u64; 128]]) {
    assert_eq!(memory.len(), CACHE_SIZE / 1024);
    assert!(std::is_x86_feature_detected!("avx2"));
    // SAFETY: exact cache slice size and AVX2 support checked for this call.
    unsafe { fill_memory(memory) }
}
fn fill_ssse3(memory: &mut [[u64; 128]]) {
    assert_eq!(memory.len(), CACHE_SIZE / 1024);
    assert!(std::is_x86_feature_detected!("ssse3"));
    // SAFETY: exact cache slice size and SSSE3 support checked for this call.
    unsafe { super::argon2_ssse3::fill_memory(memory) }
}

#[inline]
#[target_feature(enable = "avx2")]
#[allow(unused_unsafe)] // AVX2 intrinsics require unsafe on Rust 1.88.
unsafe fn blamka(a: __m256i, b: __m256i) -> __m256i {
    // SAFETY: register-only AVX2 operations under the matching target feature.
    unsafe {
        let product = _mm256_mul_epu32(a, b);
        _mm256_add_epi64(a, _mm256_add_epi64(b, _mm256_add_epi64(product, product)))
    }
}
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn g(mut a: __m256i, mut b: __m256i, mut c: __m256i, mut d: __m256i) -> [__m256i; 4] {
    // SAFETY: all operations and helper calls use AVX2 registers, no pointers.
    unsafe {
        a = blamka(a, b);
        d = _mm256_shuffle_epi32::<0xb1>(_mm256_xor_si256(d, a));
        c = blamka(c, d);
        b = _mm256_shuffle_epi8(
            _mm256_xor_si256(b, c),
            _mm256_setr_epi8(
                3, 4, 5, 6, 7, 0, 1, 2, 11, 12, 13, 14, 15, 8, 9, 10, 3, 4, 5, 6, 7, 0, 1, 2, 11,
                12, 13, 14, 15, 8, 9, 10,
            ),
        );
        a = blamka(a, b);
        d = _mm256_shuffle_epi8(
            _mm256_xor_si256(d, a),
            _mm256_setr_epi8(
                2, 3, 4, 5, 6, 7, 0, 1, 10, 11, 12, 13, 14, 15, 8, 9, 2, 3, 4, 5, 6, 7, 0, 1, 10,
                11, 12, 13, 14, 15, 8, 9,
            ),
        );
        c = blamka(c, d);
        let x = _mm256_xor_si256(b, c);
        b = _mm256_xor_si256(_mm256_srli_epi64::<63>(x), _mm256_add_epi64(x, x));
        [a, b, c, d]
    }
}
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn round<const COLUMN: bool>(mut v: [__m256i; 8]) -> [__m256i; 8] {
    // SAFETY: fixed-size local register arrays; AVX2 is enabled for every helper.
    unsafe {
        [v[0], v[2], v[4], v[6]] = g(v[0], v[2], v[4], v[6]);
        [v[1], v[3], v[5], v[7]] = g(v[1], v[3], v[5], v[7]);
        if COLUMN {
            let b0 = _mm256_blend_epi32::<0xcc>(v[2], v[3]);
            let b1 = _mm256_blend_epi32::<0x33>(v[2], v[3]);
            v[3] = _mm256_permute4x64_epi64::<0xb1>(b0);
            v[2] = _mm256_permute4x64_epi64::<0xb1>(b1);
            v.swap(4, 5);
            let d0 = _mm256_blend_epi32::<0xcc>(v[6], v[7]);
            let d1 = _mm256_blend_epi32::<0x33>(v[6], v[7]);
            v[6] = _mm256_permute4x64_epi64::<0xb1>(d0);
            v[7] = _mm256_permute4x64_epi64::<0xb1>(d1);
        } else {
            for i in [0, 1] {
                v[2 + i] = _mm256_permute4x64_epi64::<0x39>(v[2 + i]);
                v[4 + i] = _mm256_permute4x64_epi64::<0x4e>(v[4 + i]);
                v[6 + i] = _mm256_permute4x64_epi64::<0x93>(v[6 + i]);
            }
        }
        [v[0], v[2], v[4], v[6]] = g(v[0], v[2], v[4], v[6]);
        [v[1], v[3], v[5], v[7]] = g(v[1], v[3], v[5], v[7]);
        if COLUMN {
            let b0 = _mm256_blend_epi32::<0xcc>(v[2], v[3]);
            let b1 = _mm256_blend_epi32::<0x33>(v[2], v[3]);
            v[2] = _mm256_permute4x64_epi64::<0xb1>(b0);
            v[3] = _mm256_permute4x64_epi64::<0xb1>(b1);
            v.swap(4, 5);
            let d0 = _mm256_blend_epi32::<0x33>(v[6], v[7]);
            let d1 = _mm256_blend_epi32::<0xcc>(v[6], v[7]);
            v[6] = _mm256_permute4x64_epi64::<0xb1>(d0);
            v[7] = _mm256_permute4x64_epi64::<0xb1>(d1);
        } else {
            for i in [0, 1] {
                v[2 + i] = _mm256_permute4x64_epi64::<0x93>(v[2 + i]);
                v[4 + i] = _mm256_permute4x64_epi64::<0x4e>(v[4 + i]);
                v[6 + i] = _mm256_permute4x64_epi64::<0x39>(v[6 + i]);
            }
        }
        v
    }
}
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn block_prefetch<const PREFETCH: bool>(
    state: &mut [__m256i; 32],
    reference: *const u64,
    output: *mut u64,
    with_xor: bool,
    next_area: usize,
    next_beginning: usize,
    base: *const [u64; 128],
) {
    // SAFETY: callers provide two complete 1024-byte blocks and an initialized
    // 1024-byte state. All input words are read before output is overwritten.
    // When PREFETCH && next_area != 0, base spans the complete cache and
    // next_area/next_beginning describe the next item in the current segment.
    unsafe {
        let mut xy = [_mm256_setzero_si256(); 32];
        for i in 0..32 {
            state[i] = _mm256_xor_si256(state[i], _mm256_loadu_si256(reference.add(i * 4).cast()));
            xy[i] = if with_xor {
                _mm256_xor_si256(state[i], _mm256_loadu_si256(output.add(i * 4).cast()))
            } else {
                state[i]
            };
        }
        for i in 0..4 {
            let j = 8 * i;
            let v = round::<false>([
                state[j],
                state[j + 4],
                state[j + 1],
                state[j + 5],
                state[j + 2],
                state[j + 6],
                state[j + 3],
                state[j + 7],
            ]);
            for (dst, src) in [j, j + 4, j + 1, j + 5, j + 2, j + 6, j + 3, j + 7]
                .into_iter()
                .zip(v)
            {
                state[dst] = src;
            }
        }
        // Columns are independent. Once column zero is complete, its first
        // word (including feed-forward) determines the next reference address.
        // Start fetching that block while the other three columns are computed.
        let first = round::<true>(std::array::from_fn(|j| state[4 * j]));
        if PREFETCH && next_area != 0 {
            let random = _mm256_extract_epi64::<0>(_mm256_xor_si256(first[0], xy[0])) as u32;
            let squared = (u64::from(random) * u64::from(random)) >> 32;
            let relative = next_area - 1 - ((next_area as u64 * squared) >> 32) as usize;
            let reference = (next_beginning + relative) & (CACHE_SIZE / 1024 - 1);
            // The mask selects a complete 1 KiB block within the cache; all
            // sixteen hinted cache lines stay within that same block.
            let next = base.add(reference).cast::<u8>();
            for line in 0..16 {
                _mm_prefetch(next.add(line * 64).cast(), _MM_HINT_T0);
            }
        }
        for (j, src) in first.into_iter().enumerate() {
            state[4 * j] = src;
        }
        for i in 1..4 {
            let v = round::<true>(std::array::from_fn(|j| state[i + 4 * j]));
            for (j, src) in v.into_iter().enumerate() {
                state[i + 4 * j] = src;
            }
        }
        for i in 0..32 {
            state[i] = _mm256_xor_si256(state[i], xy[i]);
            _mm256_storeu_si256(output.add(i * 4).cast(), state[i]);
        }
    }
}
#[cfg(test)]
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn block(
    state: &mut [__m256i; 32],
    reference: *const u64,
    output: *mut u64,
    with_xor: bool,
) {
    // SAFETY: the test caller provides complete blocks and state. PREFETCH is
    // false, so the null cache base and zero look-ahead metadata are unused.
    unsafe {
        block_prefetch::<false>(state, reference, output, with_xor, 0, 0, std::ptr::null());
    }
}
#[target_feature(enable = "avx2")]
unsafe fn fill_memory(memory: &mut [[u64; 128]]) {
    const BLOCKS: usize = CACHE_SIZE / 1024;
    const SEGMENT: usize = BLOCKS / 4;
    assert_eq!(memory.len(), BLOCKS);
    // SAFETY: fixed-size cache verified above. Reference selection is the same
    // bounded one-lane Argon2d indexing used by the portable core. Pointers are
    // derived from this exclusive allocation and never retained past the call.
    unsafe {
        let base = memory.as_mut_ptr();
        let mut state =
            std::array::from_fn(|i| _mm256_loadu_si256(memory[1].as_ptr().add(i * 4).cast()));
        for pass in 0..3 {
            for slice in 0..4 {
                for index in if pass == 0 && slice == 0 { 2 } else { 0 }..SEGMENT {
                    let current = slice * SEGMENT + index;
                    let random = _mm256_extract_epi64::<0>(state[0]) as u32;
                    let area = if pass == 0 {
                        slice * SEGMENT + index - 1
                    } else {
                        BLOCKS - SEGMENT + index - 1
                    };
                    let squared = (u64::from(random) * u64::from(random)) >> 32;
                    let relative = area - 1 - ((area as u64 * squared) >> 32) as usize;
                    let beginning = if pass == 0 { 0 } else { (slice + 1) * SEGMENT };
                    let reference = (beginning + relative) & (BLOCKS - 1);
                    block_prefetch::<true>(
                        &mut state,
                        base.add(reference).cast(),
                        base.add(current).cast(),
                        pass != 0,
                        // The next segment has a different reference area.
                        // Skip its look-ahead; the normal loop selects it.
                        if index + 1 == SEGMENT { 0 } else { area + 1 },
                        beginning,
                        base,
                    );
                }
            }
        }
    }
}

pub(crate) fn available(force: Option<u32>) -> bool {
    match force {
        None | Some(0) | Some(96) => true,
        Some(32) => std::is_x86_feature_detected!("ssse3"),
        Some(64) => std::is_x86_feature_detected!("avx2"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compression_matches_scalar() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let mut random = 0x123456789abcdef0u64;
        for _ in 0..256 {
            let mut next = || {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                random
            };
            let prev = std::array::from_fn(|_| next());
            let reference = std::array::from_fn(|_| next());
            let old = std::array::from_fn(|_| next());
            for with_xor in [false, true] {
                let expected = crate::randomx_core::cache::fill_block(
                    &prev,
                    &reference,
                    with_xor.then_some(&old),
                );
                let mut output = old;
                // SAFETY: AVX2 checked above; all three blocks contain 128 words.
                unsafe {
                    let mut state = std::array::from_fn(|i| {
                        _mm256_loadu_si256(prev.as_ptr().add(i * 4).cast())
                    });
                    block(
                        &mut state,
                        reference.as_ptr(),
                        output.as_mut_ptr(),
                        with_xor,
                    );
                }
                assert_eq!(output, expected);
            }
        }
    }
    #[test]
    #[ignore = "compares complete 256 MiB scalar and AVX2 caches for two keys"]
    fn complete_cache_matches_scalar() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let mut a = cache(b"test key 000", Some(64), Default::default()).unwrap();
        let pointer = a.words().as_ptr();
        for (i, key) in [b"test key 000".as_slice(), b"another seed"]
            .into_iter()
            .enumerate()
        {
            if i != 0 {
                a.reinitialize(key).unwrap();
            }
            let b = Cache::new(key).unwrap();
            assert_eq!(a.words(), b.words());
            assert_eq!(a.words().as_ptr(), pointer);
            assert_eq!(a.dataset_item(12345), b.dataset_item(12345));
        }
    }
}
