// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
// SSSE3 BlaMka port. Original Argon2 code by Dinu, Khovratovich,
// Aumasson and Neves is CC0; see the RandomX core LICENSE.
use crate::randomx_core::cache::CACHE_SIZE;
use std::arch::x86_64::*;
#[inline]
#[target_feature(enable = "ssse3")]
unsafe fn g_pair(mut v: [__m128i; 8]) -> [__m128i; 8] {
    // Interleave the two independent G chains to hide multiplication latency.
    // Keep XOR before rotation: distributing a shuffle over XOR costs an extra
    // shuffle, which LLVM can otherwise do to the intrinsic implementation.
    // SAFETY: SSSE3 is enabled; all operands are complete XMM registers. The
    // assembly touches no memory, stack or flags. Early-clobber temporaries
    // cannot overlap live state or either read-only rotation mask.
    unsafe {
        let r24 = _mm_setr_epi8(3, 4, 5, 6, 7, 0, 1, 2, 11, 12, 13, 14, 15, 8, 9, 10);
        let r16 = _mm_setr_epi8(2, 3, 4, 5, 6, 7, 0, 1, 10, 11, 12, 13, 14, 15, 8, 9);
        std::arch::asm!(
            // a += b + 2 * low32(a) * low32(b); d = rotr(d ^ a, 32).
            "movdqa {t0}, {a0}",
            "movdqa {t1}, {a1}",
            "pmuludq {t0}, {b0}",
            "pmuludq {t1}, {b1}",
            "paddq {a0}, {b0}",
            "paddq {a1}, {b1}",
            "paddq {t0}, {t0}",
            "paddq {t1}, {t1}",
            "paddq {a0}, {t0}",
            "paddq {a1}, {t1}",
            "pxor {d0}, {a0}",
            "pxor {d1}, {a1}",
            "pshufd {d0}, {d0}, 177",
            "pshufd {d1}, {d1}, 177",
            // c += d + 2 * low32(c) * low32(d); b = rotr(b ^ c, 24).
            "movdqa {t0}, {c0}",
            "movdqa {t1}, {c1}",
            "pmuludq {t0}, {d0}",
            "pmuludq {t1}, {d1}",
            "paddq {c0}, {d0}",
            "paddq {c1}, {d1}",
            "paddq {t0}, {t0}",
            "paddq {t1}, {t1}",
            "paddq {c0}, {t0}",
            "paddq {c1}, {t1}",
            "pxor {b0}, {c0}",
            "pxor {b1}, {c1}",
            "pshufb {b0}, {r24}",
            "pshufb {b1}, {r24}",
            // Repeat with rotations 16 and 63 (rotate-left by one).
            "movdqa {t0}, {a0}",
            "movdqa {t1}, {a1}",
            "pmuludq {t0}, {b0}",
            "pmuludq {t1}, {b1}",
            "paddq {a0}, {b0}",
            "paddq {a1}, {b1}",
            "paddq {t0}, {t0}",
            "paddq {t1}, {t1}",
            "paddq {a0}, {t0}",
            "paddq {a1}, {t1}",
            "pxor {d0}, {a0}",
            "pxor {d1}, {a1}",
            "pshufb {d0}, {r16}",
            "pshufb {d1}, {r16}",
            "movdqa {t0}, {c0}",
            "movdqa {t1}, {c1}",
            "pmuludq {t0}, {d0}",
            "pmuludq {t1}, {d1}",
            "paddq {c0}, {d0}",
            "paddq {c1}, {d1}",
            "paddq {t0}, {t0}",
            "paddq {t1}, {t1}",
            "paddq {c0}, {t0}",
            "paddq {c1}, {t1}",
            "pxor {b0}, {c0}",
            "pxor {b1}, {c1}",
            "movdqa {t0}, {b0}",
            "movdqa {t1}, {b1}",
            "psrlq {t0}, 63",
            "psrlq {t1}, 63",
            "paddq {b0}, {b0}",
            "paddq {b1}, {b1}",
            "pxor {b0}, {t0}",
            "pxor {b1}, {t1}",
            a0 = inout(xmm_reg) v[0],
            a1 = inout(xmm_reg) v[1],
            b0 = inout(xmm_reg) v[2],
            b1 = inout(xmm_reg) v[3],
            c0 = inout(xmm_reg) v[4],
            c1 = inout(xmm_reg) v[5],
            d0 = inout(xmm_reg) v[6],
            d1 = inout(xmm_reg) v[7],
            r24 = in(xmm_reg) r24, r16 = in(xmm_reg) r16,
            t0 = out(xmm_reg) _, t1 = out(xmm_reg) _,
            options(pure, nomem, nostack, preserves_flags),
        );
        v
    }
}

#[inline]
#[target_feature(enable = "ssse3")]
unsafe fn round(mut v: [__m128i; 8]) -> [__m128i; 8] {
    // SAFETY: register-only operations under the SSSE3 feature.
    unsafe {
        v = g_pair(v);
        let b = _mm_alignr_epi8::<8>(v[3], v[2]);
        v[3] = _mm_alignr_epi8::<8>(v[2], v[3]);
        v[2] = b;
        v.swap(4, 5);
        let d = _mm_alignr_epi8::<8>(v[6], v[7]);
        v[7] = _mm_alignr_epi8::<8>(v[7], v[6]);
        v[6] = d;
        v = g_pair(v);
        let b = _mm_alignr_epi8::<8>(v[2], v[3]);
        v[3] = _mm_alignr_epi8::<8>(v[3], v[2]);
        v[2] = b;
        v.swap(4, 5);
        let d = _mm_alignr_epi8::<8>(v[7], v[6]);
        v[7] = _mm_alignr_epi8::<8>(v[6], v[7]);
        v[6] = d;
        v
    }
}
#[inline]
#[target_feature(enable = "ssse3")]
unsafe fn block_prefetch<const PREFETCH: bool>(
    state: &mut [__m128i; 64],
    reference: *const u64,
    output: *mut u64,
    with_xor: bool,
    next_area: usize,
    next_beginning: usize,
    base: *const [u64; 128],
) {
    // SAFETY: callers provide complete 1024-byte blocks and state. Read all
    // reference/output words before writing the output, permitting aliasing.
    // When PREFETCH && next_area != 0, base covers the complete cache and
    // next_area/next_beginning describe the next block in this segment.
    unsafe {
        let mut xy = [_mm_setzero_si128(); 64];
        // XOR/reference loading feeds each row round directly; do not write
        // the unpermuted state only to reload it in a second traversal.
        for row in 0..8 {
            let v = std::array::from_fn(|lane| {
                let i = row * 8 + lane;
                let v = _mm_xor_si128(state[i], _mm_loadu_si128(reference.add(i * 2).cast()));
                xy[i] = if with_xor {
                    _mm_xor_si128(v, _mm_loadu_si128(output.add(i * 2).cast()))
                } else {
                    v
                };
                v
            });
            let v = round(v);
            state[row * 8..row * 8 + 8].copy_from_slice(&v);
        }
        // All reference and old output words are now captured. Finish each
        // column directly into state and output, avoiding another traversal.
        // The first completed column determines the next reference address.
        // Fetch that block while the other seven independent columns run.
        let first = round(std::array::from_fn(|lane| state[lane * 8]));
        if PREFETCH && next_area != 0 {
            let random = _mm_cvtsi128_si64(_mm_xor_si128(first[0], xy[0])) as u32;
            let squared = (u64::from(random) * u64::from(random)) >> 32;
            let relative = next_area - 1 - ((next_area as u64 * squared) >> 32) as usize;
            let reference = (next_beginning + relative) & (CACHE_SIZE / 1024 - 1);
            // The mask selects a complete 1 KiB block; all hints are in bounds.
            let next = base.add(reference).cast::<u8>();
            for line in 0..16 {
                _mm_prefetch(next.add(line * 64).cast(), _MM_HINT_T0);
            }
        }
        for (lane, value) in first.into_iter().enumerate() {
            let i = lane * 8;
            state[i] = _mm_xor_si128(value, xy[i]);
            _mm_storeu_si128(output.add(i * 2).cast(), state[i]);
        }
        for column in 1..8 {
            let v = round(std::array::from_fn(|lane| state[column + lane * 8]));
            for (lane, value) in v.into_iter().enumerate() {
                let i = column + lane * 8;
                state[i] = _mm_xor_si128(value, xy[i]);
                _mm_storeu_si128(output.add(i * 2).cast(), state[i]);
            }
        }
    }
}
#[cfg(test)]
#[inline]
#[target_feature(enable = "ssse3")]
unsafe fn block(
    state: &mut [__m128i; 64],
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
#[target_feature(enable = "ssse3")]
pub(super) unsafe fn fill_memory(memory: &mut [[u64; 128]]) {
    const BLOCKS: usize = CACHE_SIZE / 1024;
    const SEGMENT: usize = BLOCKS / 4;
    assert_eq!(memory.len(), BLOCKS);
    // SAFETY: fixed-size cache verified above. Reference selection is the same
    // bounded one-lane Argon2d indexing used by the portable core. Pointers are
    // derived from this exclusive allocation and never retained past the call.
    unsafe {
        let base = memory.as_mut_ptr();
        let mut state =
            std::array::from_fn(|i| _mm_loadu_si128(memory[1].as_ptr().add(i * 2).cast()));
        for pass in 0..3 {
            for slice in 0..4 {
                for index in if pass == 0 && slice == 0 { 2 } else { 0 }..SEGMENT {
                    let current = slice * SEGMENT + index;
                    let random = _mm_cvtsi128_si64(state[0]) as u32;
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
                        // Crossing a segment changes the reference area.
                        if index + 1 == SEGMENT { 0 } else { area + 1 },
                        beginning,
                        base,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::argon2::cache;
    use super::*;
    use crate::randomx_core::cache::Cache;
    #[test]
    fn compression_matches_scalar() {
        if !std::is_x86_feature_detected!("ssse3") {
            return;
        }
        let mut random = 0x123456789abcdef0u64;
        for case in 0..260 {
            let mut next = || {
                // Cover low-half multiplication overflow and rotation edges,
                // followed by 256 independent randomized block triples.
                if case < 4 {
                    return [0, u64::MAX, 1u64 << 32, u64::from(u32::MAX)][case];
                }
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
                // SAFETY: SSSE3 checked above; all three blocks contain 128 words.
                unsafe {
                    let mut state =
                        std::array::from_fn(|i| _mm_loadu_si128(prev.as_ptr().add(i * 2).cast()));
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
    #[ignore = "compares complete 256 MiB scalar and SSSE3 caches for two keys"]
    fn complete_cache_matches_scalar() {
        if !std::is_x86_feature_detected!("ssse3") {
            return;
        }
        let mut a = cache(b"test key 000", Some(32), Default::default()).unwrap();
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
