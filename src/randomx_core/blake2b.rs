// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
// Ported from the pinned RandomX BLAKE2b implementation; see LICENSE here.
// Argon2 H' originates from the Argon2 reference implementation (CC0).

//! Unkeyed BLAKE2b and Argon2 H', the variants used by RandomX.
//! Fixed-size state; no heap allocation or platform-specific instructions.

use core::fmt;

const IV: [u64; 8] = [
    0x6a09e667f3bcc908,
    0xbb67ae8584caa73b,
    0x3c6ef372fe94f82b,
    0xa54ff53a5f1d36f1,
    0x510e527fade682d1,
    0x9b05688c2b3e6c1f,
    0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
];
const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidOutputLength;
impl fmt::Display for InvalidOutputLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid BLAKE2b output length")
    }
}
impl std::error::Error for InvalidOutputLength {}

#[derive(Clone)]
pub struct Blake2b {
    h: [u64; 8],
    count: u128,
    buffer: [u8; 128],
    buffered: usize,
    output_len: usize,
}

impl Blake2b {
    /// Digest length is part of the hash parameters, not a truncation of H512.
    pub fn new(output_len: usize) -> Result<Self, InvalidOutputLength> {
        if !(1..=64).contains(&output_len) {
            return Err(InvalidOutputLength);
        }
        let mut h = IV;
        h[0] ^= 0x01010000 | output_len as u64;
        Ok(Self {
            h,
            count: 0,
            buffer: [0; 128],
            buffered: 0,
            output_len,
        })
    }

    pub fn update(&mut self, mut input: &[u8]) {
        if input.is_empty() {
            return;
        }
        if self.buffered != 0 {
            let take = input.len().min(128 - self.buffered);
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&input[..take]);
            self.buffered += take;
            input = &input[take..];
            if input.is_empty() {
                return;
            }
            self.count = self.count.wrapping_add(128);
            compress(&mut self.h, &self.buffer, self.count, false);
            self.buffered = 0;
        }
        // Keep the final complete block for finalize's last-block flag.
        while input.len() > 128 {
            self.count = self.count.wrapping_add(128);
            compress(
                &mut self.h,
                input[..128].try_into().unwrap(),
                self.count,
                false,
            );
            input = &input[128..];
        }
        self.buffer[..input.len()].copy_from_slice(input);
        self.buffered = input.len();
    }

    /// Consumes the state so callers cannot update or finalize it twice.
    /// Output must have exactly the length selected by `new`.
    pub fn finalize(mut self, output: &mut [u8]) -> Result<(), InvalidOutputLength> {
        if output.len() != self.output_len {
            return Err(InvalidOutputLength);
        }
        self.count = self.count.wrapping_add(self.buffered as u128);
        self.buffer[self.buffered..].fill(0);
        compress(&mut self.h, &self.buffer, self.count, true);
        for (chunk, word) in output.chunks_mut(8).zip(self.h) {
            chunk.copy_from_slice(&word.to_le_bytes()[..chunk.len()]);
        }
        Ok(())
    }
}

#[inline]
fn mix(v: &mut [u64; 16], lanes: [usize; 4], x: u64, y: u64) {
    let [a, b, c, d] = lanes;
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(24);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

fn compress(h: &mut [u64; 8], block: &[u8; 128], count: u128, last: bool) {
    let mut words = [0; 16];
    for (word, bytes) in words.iter_mut().zip(block.as_chunks::<8>().0) {
        *word = u64::from_le_bytes(*bytes);
    }
    let mut v = [0; 16];
    v[..8].copy_from_slice(h);
    v[8..].copy_from_slice(&IV);
    v[12] ^= count as u64;
    v[13] ^= (count >> 64) as u64;
    if last {
        v[14] = !v[14];
    }
    for round in 0..12 {
        let s = &SIGMA[round % 10];
        mix(&mut v, [0, 4, 8, 12], words[s[0]], words[s[1]]);
        mix(&mut v, [1, 5, 9, 13], words[s[2]], words[s[3]]);
        mix(&mut v, [2, 6, 10, 14], words[s[4]], words[s[5]]);
        mix(&mut v, [3, 7, 11, 15], words[s[6]], words[s[7]]);
        mix(&mut v, [0, 5, 10, 15], words[s[8]], words[s[9]]);
        mix(&mut v, [1, 6, 11, 12], words[s[10]], words[s[11]]);
        mix(&mut v, [2, 7, 8, 13], words[s[12]], words[s[13]]);
        mix(&mut v, [3, 4, 9, 14], words[s[14]], words[s[15]]);
    }
    for i in 0..8 {
        h[i] ^= v[i] ^ v[i + 8];
    }
}

pub fn hash(input: &[u8], output: &mut [u8]) -> Result<(), InvalidOutputLength> {
    let mut state = Blake2b::new(output.len())?;
    state.update(input);
    state.finalize(output)
}

pub fn hash_512(input: &[u8]) -> [u8; 64] {
    let mut output = [0; 64];
    hash(input, &mut output).expect("64-byte digest length is valid");
    output
}

/// Argon2 H': prefix the output length, then expand using chained BLAKE2b.
/// This is distinct from ordinary BLAKE2b even when output is at most 64 bytes.
pub fn long_hash(input: &[u8], output: &mut [u8]) -> Result<(), InvalidOutputLength> {
    let length = u32::try_from(output.len()).map_err(|_| InvalidOutputLength)?;
    if length == 0 {
        return Err(InvalidOutputLength);
    }
    let mut state = Blake2b::new(output.len().min(64))?;
    state.update(&length.to_le_bytes());
    state.update(input);
    if output.len() <= 64 {
        return state.finalize(output);
    }
    let mut block = [0; 64];
    state.finalize(&mut block)?;
    output[..32].copy_from_slice(&block[..32]);
    let mut remaining = &mut output[32..];
    while remaining.len() > 64 {
        block = hash_512(&block);
        remaining[..32].copy_from_slice(&block[..32]);
        remaining = &mut remaining[32..];
    }
    hash(&block, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digest_and_output_length_validation() {
        let hex: String = hash_512(b"abc")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923");
        assert!(Blake2b::new(0).is_err());
        assert!(Blake2b::new(65).is_err());
        let mut untouched = [0xa5; 31];
        assert!(Blake2b::new(32).unwrap().finalize(&mut untouched).is_err());
        assert_eq!(untouched, [0xa5; 31]);
        assert!(long_hash(b"", &mut []).is_err());
    }

    #[test]
    fn updates_preserve_the_last_complete_block() {
        let input = [0x47; 513];
        for size in [0, 1, 127, 128, 129, 255, 256, 257, 512, 513] {
            let expected = hash_512(&input[..size]);
            for split in 0..=size {
                let mut state = Blake2b::new(64).unwrap();
                state.update(&input[..split]);
                state.update(&[]);
                state.update(&input[split..size]);
                state.update(&[]);
                let mut actual = [0; 64];
                state.finalize(&mut actual).unwrap();
                assert_eq!(actual, expected, "size={size}, split={split}");
            }
        }
    }
}
