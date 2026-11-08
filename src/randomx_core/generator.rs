// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
// Port of RandomX's Blake2Generator; see LICENSE here.

use super::blake2b::hash_512;

/// Deterministic byte stream used to generate superscalar programs.
#[derive(Clone)]
pub struct Blake2Generator {
    data: [u8; 64],
    index: usize,
}

impl Blake2Generator {
    /// Only the first 60 seed bytes are used; nonce occupies the final 4 bytes.
    pub fn new(seed: &[u8], nonce: u32) -> Self {
        let mut data = [0; 64];
        let count = seed.len().min(60);
        data[..count].copy_from_slice(&seed[..count]);
        data[60..].copy_from_slice(&nonce.to_le_bytes());
        Self { data, index: 64 }
    }

    fn refill(&mut self, count: usize) {
        if self.index + count > 64 {
            // An integer read discards any remaining 1–3 bytes. It must not
            // combine bytes from two hash blocks.
            self.data = hash_512(&self.data);
            self.index = 0;
        }
    }

    pub fn next_byte(&mut self) -> u8 {
        self.refill(1);
        let value = self.data[self.index];
        self.index += 1;
        value
    }

    pub fn next_u32(&mut self) -> u32 {
        self.refill(4);
        let value = u32::from_le_bytes(self.data[self.index..self.index + 4].try_into().unwrap());
        self.index += 4;
        value
    }
}
