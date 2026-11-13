// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
// RandomX AES constructions, ported from aes_hash.cpp; see LICENSE here.

//! Software AES rounds and RandomX's four-lane constructions. These reduced
//! round constructions are specific to RandomX, not general encryption APIs.

pub(crate) const fn multiply(mut a: u8, mut b: u8) -> u8 {
    let mut value = 0;
    while b != 0 {
        if b & 1 != 0 {
            value ^= a;
        }
        a = (a << 1) ^ if a & 128 != 0 { 0x1b } else { 0 };
        b >>= 1;
    }
    value
}

const fn substitution() -> ([u8; 256], [u8; 256]) {
    let mut forward = [0; 256];
    let mut inverse = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut value = 1;
        let mut power = 0;
        while power < 254 {
            value = multiply(value, i as u8);
            power += 1;
        }
        let substituted = value
            ^ value.rotate_left(1)
            ^ value.rotate_left(2)
            ^ value.rotate_left(3)
            ^ value.rotate_left(4)
            ^ 0x63;
        forward[i] = substituted;
        inverse[substituted as usize] = i as u8;
        i += 1;
    }
    (forward, inverse)
}
pub(crate) const SBOX: ([u8; 256], [u8; 256]) = substitution();

const fn tables(decrypt: bool) -> [[u32; 256]; 4] {
    let mut table = [[0; 256]; 4];
    let mut i = 0;
    while i < 256 {
        let s = if decrypt { SBOX.1[i] } else { SBOX.0[i] };
        let value = u32::from_le_bytes(if decrypt {
            [
                multiply(s, 14),
                multiply(s, 9),
                multiply(s, 13),
                multiply(s, 11),
            ]
        } else {
            [multiply(s, 2), s, s, multiply(s, 3)]
        });
        let mut lane = 0;
        while lane < 4 {
            table[lane][i] = value.rotate_left(lane as u32 * 8);
            lane += 1;
        }
        i += 1;
    }
    table
}
// Keep every 1 KiB lookup lane on cache-line boundaries. No lane then spans
// an extra line when software AES shares L1 with the RandomX scratchpad.
#[repr(align(64))]
pub(crate) struct Tables([[u32; 256]; 4]);
impl std::ops::Deref for Tables {
    type Target = [[u32; 256]; 4];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
pub(crate) static ENCODE: Tables = Tables(tables(false));
pub(crate) static DECODE: Tables = Tables(tables(true));

/// One RandomX AESENC/AESDEC round with compile-time-generated lookup tables.
/// The input is public RandomX state; this is not a constant-time cipher API.
#[inline(always)]
pub fn round(state: [u8; 16], key: [u8; 16], decrypt: bool) -> [u8; 16] {
    if decrypt {
        round_table::<true>(state, key)
    } else {
        round_table::<false>(state, key)
    }
}
#[inline(always)]
fn round_table<const DECRYPT: bool>(state: [u8; 16], key: [u8; 16]) -> [u8; 16] {
    let table = if DECRYPT { &DECODE } else { &ENCODE };
    let mut out = [0; 16];
    for column in 0..4 {
        let c1 = (column + if DECRYPT { 3 } else { 1 }) & 3;
        let c3 = (column + if DECRYPT { 1 } else { 3 }) & 3;
        let value = table[0][state[4 * column] as usize]
            ^ table[1][state[4 * c1 + 1] as usize]
            ^ table[2][state[4 * ((column + 2) & 3) + 2] as usize]
            ^ table[3][state[4 * c3 + 3] as usize]
            ^ u32::from_le_bytes(key[column * 4..column * 4 + 4].try_into().unwrap());
        out[column * 4..column * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    out
}

#[cfg(test)]
/// One AESENC/AESDEC round; the decryption key is added after inverse mixing,
/// exactly as in the CPU instructions (no implicit inverse-key transform).
fn round_reference(state: [u8; 16], key: [u8; 16], decrypt: bool) -> [u8; 16] {
    let mut output = [0; 16];
    for column in 0..4 {
        let mut x = [0; 4];
        for row in 0..4 {
            let shifted = if decrypt {
                (column + 4 - row) % 4
            } else {
                (column + row) % 4
            };
            let byte = usize::from(state[shifted * 4 + row]);
            x[row] = if decrypt { SBOX.1[byte] } else { SBOX.0[byte] };
        }
        for row in 0..4 {
            let [a, b, c, d] = [x[row], x[(row + 1) % 4], x[(row + 2) % 4], x[(row + 3) % 4]];
            output[column * 4 + row] = key[column * 4 + row]
                ^ if decrypt {
                    multiply(a, 14) ^ multiply(b, 11) ^ multiply(c, 13) ^ multiply(d, 9)
                } else {
                    multiply(a, 2) ^ multiply(b, 3) ^ c ^ d
                };
        }
    }
    output
}

#[derive(Clone)]
pub struct Aes {
    pub(crate) hash_state: [[u8; 16]; 4],
    pub(crate) extra: [[u8; 16]; 2],
    pub(crate) fill_keys: [[u8; 16]; 4],
    pub(crate) program_keys: [[u8; 16]; 8],
}
impl Default for Aes {
    fn default() -> Self {
        // Fixed RandomX constants: BLAKE2b of the five specification labels.
        // Keep keys local to each backend, but avoid re-deriving them for every
        // VM. The derivation test below checks all bytes and their lane order.
        Self {
            hash_state: [
                0xd7983aadcc82db479fa856de92b52c0du128.to_le_bytes(),
                0xace78057f59e125a15c7b798338d996eu128.to_le_bytes(),
                0xe8a07ce45079506bae62c7d06a770017u128.to_le_bytes(),
                0x7e99494879a1000507ad828d630a240cu128.to_le_bytes(),
            ],
            extra: [
                0x0689020190dc56bf8b24949ff6fa8389u128.to_le_bytes(),
                0xed18f99bee1043c651f4e03c61b263d1u128.to_le_bytes(),
            ],
            fill_keys: [
                0xb4f44917dbb5552b627166096daca553u128.to_le_bytes(),
                0x0da1dc4e1725d378846a710d6d7caf07u128.to_le_bytes(),
                0x3e20e345f4c0794f9f947ec63f1262f1u128.to_le_bytes(),
                0x4916915416314c88b1ba317c6aef8135u128.to_le_bytes(),
            ],
            program_keys: [
                0x99e5d23f2f546d2bd1833ddb6421aaddu128.to_le_bytes(),
                0xa5dfcde506f79d53b6913f55b20e3450u128.to_le_bytes(),
                0x171c02bf0aa4679f515e7baf5c3ed904u128.to_le_bytes(),
                0xd8ded291cd673785e78f5d0885623763u128.to_le_bytes(),
                0x229effb43d518b6de3d6a7a6b5826f73u128.to_le_bytes(),
                0xb272b7d2e9024d4e9c10b3d9c7566bf3u128.to_le_bytes(),
                0xf63befa72ba9660af765a38bf273c9e7u128.to_le_bytes(),
                0xc0b0762d0c06d1fd915839de7a7cd609u128.to_le_bytes(),
            ],
        }
    }
}

fn lanes(bytes: &[u8; 64]) -> [[u8; 16]; 4] {
    bytes.as_chunks::<16>().0.try_into().unwrap()
}
fn flatten(lanes: [[u8; 16]; 4]) -> [u8; 64] {
    std::array::from_fn(|i| lanes[i / 16][i % 16])
}

impl Aes {
    pub fn fill(&self, state: &mut [u8; 64], output: &mut [u8]) {
        assert!(output.len().is_multiple_of(64));
        let mut registers = lanes(state);
        for block in output.as_chunks_mut::<64>().0 {
            for (i, register) in registers.iter_mut().enumerate() {
                *register = round(*register, self.fill_keys[i], i % 2 == 0);
            }
            *block = flatten(registers);
        }
        *state = flatten(registers);
    }

    /// Four rounds per block. Unlike fill(), the input seed is not advanced.
    pub fn program(&self, seed: &[u8; 64], output: &mut [u8]) {
        assert!(output.len().is_multiple_of(64));
        let mut registers = lanes(seed);
        for block in output.as_chunks_mut::<64>().0 {
            for step in 0..4 {
                for (i, register) in registers.iter_mut().enumerate() {
                    *register = round(*register, self.program_keys[step + (i / 2) * 4], i % 2 == 0);
                }
            }
            *block = flatten(registers);
        }
    }

    pub fn finish(&self, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
        let Some(next) = next else {
            return self.hash(input);
        };
        assert!(input.len().is_multiple_of(64));
        let mut hash = self.hash_state;
        let mut fill = lanes(next);
        for block in input.as_chunks_mut::<64>().0 {
            let keys = lanes(block);
            hash[0] = round_table::<false>(hash[0], keys[0]);
            hash[1] = round_table::<true>(hash[1], keys[1]);
            hash[2] = round_table::<false>(hash[2], keys[2]);
            hash[3] = round_table::<true>(hash[3], keys[3]);
            fill[0] = round_table::<true>(fill[0], self.fill_keys[0]);
            fill[1] = round_table::<false>(fill[1], self.fill_keys[1]);
            fill[2] = round_table::<true>(fill[2], self.fill_keys[2]);
            fill[3] = round_table::<false>(fill[3], self.fill_keys[3]);
            *block = flatten(fill);
        }
        for key in self.extra {
            hash[0] = round_table::<false>(hash[0], key);
            hash[1] = round_table::<true>(hash[1], key);
            hash[2] = round_table::<false>(hash[2], key);
            hash[3] = round_table::<true>(hash[3], key);
        }
        *next = flatten(fill);
        flatten(hash)
    }
    pub fn hash(&self, input: &[u8]) -> [u8; 64] {
        assert!(input.len().is_multiple_of(64));
        let mut registers = self.hash_state;
        for block in input.as_chunks::<64>().0 {
            let keys = lanes(block);
            for (i, register) in registers.iter_mut().enumerate() {
                *register = round(*register, keys[i], i % 2 != 0);
            }
        }
        for key in self.extra {
            for (i, register) in registers.iter_mut().enumerate() {
                *register = round(*register, key, i % 2 != 0);
            }
        }
        flatten(registers)
    }
}

/// Interchangeable RandomX AES constructions. Implementations must produce the
/// same byte stream and may use hardware instructions outside this safe core.
pub trait Backend: Send + Sync {
    fn fill(&self, state: &mut [u8; 64], output: &mut [u8]);
    fn program(&self, state: &[u8; 64], output: &mut [u8]);
    fn finish(&self, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64];
    fn mix(&self, f: &mut [u64; 8], e: &[u64; 8]) {
        for key in e.as_chunks::<2>().0 {
            let key = std::array::from_fn(|i| key[i / 8].to_le_bytes()[i % 8]);
            for (i, lane) in f.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let bytes = std::array::from_fn(|j| lane[j / 8].to_le_bytes()[j % 8]);
                let bytes = round(bytes, key, i % 2 != 0);
                lane[0] = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                lane[1] = u64::from_le_bytes(bytes[8..].try_into().unwrap());
            }
        }
    }
}
impl Backend for Aes {
    fn fill(&self, state: &mut [u8; 64], output: &mut [u8]) {
        self.fill(state, output);
    }
    fn program(&self, state: &[u8; 64], output: &mut [u8]) {
        self.program(state, output);
    }
    fn finish(&self, input: &mut [u8], next: Option<&mut [u8; 64]>) -> [u8; 64] {
        self.finish(input, next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_keys_match_specification_derivation() {
        use super::super::blake2b;
        let aes = Aes::default();
        assert_eq!(
            flatten(aes.hash_state),
            blake2b::hash_512(b"RandomX AesHash1R state")
        );
        assert_eq!(
            flatten(aes.fill_keys),
            blake2b::hash_512(b"RandomX AesGenerator1R keys")
        );
        let mut extra = [0; 32];
        blake2b::hash(b"RandomX AesHash1R xkeys", &mut extra).unwrap();
        assert_eq!(aes.extra.as_flattened(), extra);
        assert_eq!(
            aes.program_keys[..4].as_flattened(),
            blake2b::hash_512(b"RandomX AesGenerator4R keys 0-3")
        );
        assert_eq!(
            aes.program_keys[4..].as_flattened(),
            blake2b::hash_512(b"RandomX AesGenerator4R keys 4-7")
        );
    }

    #[test]
    fn table_rounds_and_fused_refill_match_reference() {
        let mut random = 0x123456789abcdef0u64;
        for _ in 0..4096 {
            let mut bytes = || {
                std::array::from_fn(|_| {
                    random ^= random << 13;
                    random ^= random >> 7;
                    random ^= random << 17;
                    random as u8
                })
            };
            let state = bytes();
            let key = bytes();
            for decrypt in [false, true] {
                assert_eq!(
                    round(state, key, decrypt),
                    round_reference(state, key, decrypt)
                );
            }
        }
        let aes = Aes::default();
        for size in [0, 64, 2176, 3200, 2 << 20] {
            let mut data = vec![0x71; size];
            let mut expected = data.clone();
            let hash = aes.hash(&data);
            let mut seed = [0x36; 64];
            let mut expected_seed = seed;
            aes.fill(&mut expected_seed, &mut expected);
            assert_eq!(aes.finish(&mut data, Some(&mut seed)), hash);
            assert_eq!(data, expected);
            assert_eq!(seed, expected_seed);
        }
    }
    #[test]
    fn upstream_aes_generator_vector() {
        let input = "6c19536eb2de31b6c0065f7f116e86f960d8af0c57210a6584c3237b9d064dc7";
        let mut state = [0; 64];
        for (i, byte) in state[..32].iter_mut().enumerate() {
            *byte = u8::from_str_radix(&input[2 * i..2 * i + 2], 16).unwrap();
        }
        let mut output = [0; 64];
        Aes::default().fill(&mut state, &mut output);
        let hex: String = output[..32].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "fa89397dd6ca422513aeadba3f124b5540324c4ad4b6db434394307a17c833ab"
        );
        assert_eq!(state, output);
    }
}
