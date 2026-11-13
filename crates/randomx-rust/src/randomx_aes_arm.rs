// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
//! Runtime-gated ARMv8 AES versions of the reference core's AES constructions.
use crate::randomx_core::aes::Aes;
use std::arch::aarch64::*;
pub(crate) struct HardwareAes {
    keys: Aes,
}
impl HardwareAes {
    pub fn new() -> Option<Self> {
        std::arch::is_aarch64_feature_detected!("aes").then(|| Self {
            keys: Aes::default(),
        })
    }
    pub fn fill(&self, state: &mut [u8; 64], output: &mut [u8]) {
        assert!(output.len().is_multiple_of(64));
        // SAFETY: construction checks ARMv8 AES; all input/output lengths are checked.
        unsafe { fill(&self.keys, state, output) }
    }
    pub fn program(&self, state: &[u8; 64], output: &mut [u8]) {
        assert!(output.len().is_multiple_of(64));
        // SAFETY: construction checks ARMv8 AES; all input/output lengths are checked.
        unsafe { program(&self.keys, state, output) }
    }
    pub fn finish(&self, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
        assert!(input.len().is_multiple_of(64));
        // SAFETY: construction checks ARMv8 AES; chunks and states have complete lanes.
        unsafe { finish(&self.keys, input, next) }
    }
}
#[target_feature(enable = "aes,neon")]
unsafe fn fill(keys: &Aes, state: &mut [u8; 64], output: &mut [u8]) {
    // SAFETY: every unaligned load/store addresses a complete 16-byte lane;
    // arrays/chunks enforce these bounds. AES is enabled for this function.
    unsafe {
        let mut s: [uint8x16_t; 4] =
            std::array::from_fn(|i| vld1q_u8(state.as_ptr().add(i * 16).cast()));
        let k: [uint8x16_t; 4] =
            std::array::from_fn(|i| vld1q_u8(keys.fill_keys[i].as_ptr().cast()));
        for b in output.as_chunks_mut::<64>().0 {
            s[0] = dec(s[0], k[0]);
            s[1] = enc(s[1], k[1]);
            s[2] = dec(s[2], k[2]);
            s[3] = enc(s[3], k[3]);
            for (i, &lane) in s.iter().enumerate() {
                vst1q_u8(b.as_mut_ptr().add(i * 16).cast(), lane);
            }
        }
        for (i, &lane) in s.iter().enumerate() {
            vst1q_u8(state.as_mut_ptr().add(i * 16).cast(), lane);
        }
    }
}
#[target_feature(enable = "aes,neon")]
unsafe fn program(keys: &Aes, state: &[u8; 64], output: &mut [u8]) {
    // SAFETY: complete lanes within checked arrays/chunks; AES is enabled.
    unsafe {
        let mut s: [uint8x16_t; 4] =
            std::array::from_fn(|i| vld1q_u8(state.as_ptr().add(i * 16).cast()));
        let k: [uint8x16_t; 8] =
            std::array::from_fn(|i| vld1q_u8(keys.program_keys[i].as_ptr().cast()));
        for b in output.as_chunks_mut::<64>().0 {
            for j in 0..4 {
                s[0] = dec(s[0], k[j]);
                s[1] = enc(s[1], k[j]);
                s[2] = dec(s[2], k[j + 4]);
                s[3] = enc(s[3], k[j + 4]);
            }
            for (i, &lane) in s.iter().enumerate() {
                vst1q_u8(b.as_mut_ptr().add(i * 16).cast(), lane);
            }
        }
    }
}
#[target_feature(enable = "aes,neon")]
unsafe fn finish(keys: &Aes, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
    // SAFETY: complete 16-byte lanes within checked arrays/chunks. Each scratch
    // chunk is loaded before refill overwrites it. AES is enabled.
    unsafe {
        let mut hash: [uint8x16_t; 4] =
            std::array::from_fn(|i| vld1q_u8(keys.hash_state[i].as_ptr().cast()));
        let mut fill = next
            .as_ref()
            .map(|n| std::array::from_fn::<_, 4, _>(|i| vld1q_u8(n.as_ptr().add(i * 16).cast())));
        let k: [uint8x16_t; 4] =
            std::array::from_fn(|i| vld1q_u8(keys.fill_keys[i].as_ptr().cast()));
        for b in input.as_chunks_mut::<64>().0 {
            let x: [uint8x16_t; 4] =
                std::array::from_fn(|i| vld1q_u8(b.as_ptr().add(i * 16).cast()));
            hash[0] = enc(hash[0], x[0]);
            hash[1] = dec(hash[1], x[1]);
            hash[2] = enc(hash[2], x[2]);
            hash[3] = dec(hash[3], x[3]);
            if let Some(s) = &mut fill {
                s[0] = dec(s[0], k[0]);
                s[1] = enc(s[1], k[1]);
                s[2] = dec(s[2], k[2]);
                s[3] = enc(s[3], k[3]);
                for (i, &lane) in s.iter().enumerate() {
                    vst1q_u8(b.as_mut_ptr().add(i * 16).cast(), lane);
                }
            }
        }
        for k in &keys.extra {
            let k = vld1q_u8(k.as_ptr().cast());
            hash[0] = enc(hash[0], k);
            hash[1] = dec(hash[1], k);
            hash[2] = enc(hash[2], k);
            hash[3] = dec(hash[3], k);
        }
        if let (Some(next), Some(s)) = (next, fill) {
            for (i, &lane) in s.iter().enumerate() {
                vst1q_u8(next.as_mut_ptr().add(i * 16).cast(), lane);
            }
        }
        let mut out = [0; 64];
        for (i, &lane) in hash.iter().enumerate() {
            vst1q_u8(out.as_mut_ptr().add(i * 16).cast(), lane);
        }
        out
    }
}

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
#[target_feature(enable = "aes,neon")]
unsafe fn mix(f: &mut [u64; 8], e: &[u64; 8]) {
    // SAFETY: complete 16-byte lanes within the two arrays; AES is enabled.
    unsafe {
        let mut lanes: [uint8x16_t; 4] =
            std::array::from_fn(|i| vld1q_u8(f.as_ptr().add(2 * i).cast()));
        for i in 0..4 {
            let key = vld1q_u8(e.as_ptr().add(2 * i).cast());
            lanes[0] = enc(lanes[0], key);
            lanes[1] = dec(lanes[1], key);
            lanes[2] = enc(lanes[2], key);
            lanes[3] = dec(lanes[3], key);
        }
        for (i, lane) in lanes.into_iter().enumerate() {
            vst1q_u8(f.as_mut_ptr().add(2 * i).cast(), lane);
        }
    }
}

#[inline]
#[target_feature(enable = "aes,neon")]
#[allow(unused_unsafe)] // Intrinsics require unsafe on the minimum Rust version.
unsafe fn enc(state: uint8x16_t, key: uint8x16_t) -> uint8x16_t {
    // SAFETY: register-only AES and NEON operations under the matching features.
    unsafe { veorq_u8(vaesmcq_u8(vaeseq_u8(state, vdupq_n_u8(0))), key) }
}
#[inline]
#[target_feature(enable = "aes,neon")]
#[allow(unused_unsafe)]
unsafe fn dec(state: uint8x16_t, key: uint8x16_t) -> uint8x16_t {
    // SAFETY: register-only AES and NEON operations under the matching features.
    unsafe { veorq_u8(vaesimcq_u8(vaesdq_u8(state, vdupq_n_u8(0))), key) }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomx_core::aes::Backend;
    #[test]
    fn hardware_matches_software() {
        let Some(aes) = HardwareAes::new() else {
            return;
        };
        let soft = Aes::default();
        for size in [0, 64, 128 + 256 * 8, 128 + 384 * 8, 2 << 20] {
            let mut seed = [7; 64];
            let mut expected_seed = seed;
            let mut buffer = vec![0; size];
            let mut expected = buffer.clone();
            aes.fill(&mut seed, &mut buffer);
            soft.fill(&mut expected_seed, &mut expected);
            assert_eq!(buffer, expected);
            assert_eq!(seed, expected_seed);
            let digest = soft.hash(&expected);
            soft.fill(&mut expected_seed, &mut expected);
            assert_eq!(aes.finish(&mut buffer, Some(&mut seed)), digest);
            assert_eq!(buffer, expected);
            assert_eq!(seed, expected_seed);
            aes.program(&seed, &mut buffer);
            soft.program(&seed, &mut expected);
            assert_eq!(buffer, expected);
        }
        let mut f = std::array::from_fn(|i| 1234567890u64.wrapping_mul(i as u64));
        let e = std::array::from_fn(|i| 975316428u64.wrapping_mul(i as u64 + 1));
        let mut expected = f;
        aes.mix(&mut f, &e);
        soft.mix(&mut expected, &e);
        assert_eq!(f, expected);
    }
}
