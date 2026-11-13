// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
// Port of RandomX's cache/dataset initialization; see LICENSE here.
// Argon2's reference compression and indexing originate from its CC0 implementation.

//! RandomX's Argon2d cache and on-demand dataset items. This is the fixed
//! RandomX configuration (v1.3, one lane, three passes), not a password API.
use super::{
    blake2b::{self, Blake2b},
    generator::Blake2Generator,
    scheduler,
    superscalar::Program,
};
use std::{collections::TryReserveError, fmt};

pub const CACHE_SIZE: usize = 256 * 1024 * 1024;
pub const DATASET_ITEMS: usize = (2147483648u64 + 33554368) as usize / 64;
const BLOCKS: usize = CACHE_SIZE / 1024;
const SEGMENT: usize = BLOCKS / 4;
const PASSES: u32 = 3;
pub(crate) const ADDS: [u64; 8] = [
    0,
    9298411001130361340,
    12065312585734608966,
    9306329213124626780,
    5281919268842080866,
    10536153434571861004,
    3398623926847679864,
    9549104520008361294,
];

#[derive(Debug)]
pub enum CacheError {
    KeyTooLong,
    Allocation(TryReserveError),
    InvalidRange,
    Thread(std::io::Error),
    Backend(std::io::Error),
    Interrupted,
}
impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyTooLong => f.write_str("RandomX key length exceeds u32"),
            Self::Allocation(error) => write!(f, "RandomX allocation failed: {error}"),
            Self::InvalidRange => f.write_str("RandomX dataset range is out of bounds"),
            Self::Thread(error) => write!(f, "RandomX initializer thread failed: {error}"),
            Self::Backend(error) => write!(f, "RandomX accelerated backend failed: {error}"),
            Self::Interrupted => f.write_str("RandomX initialization interrupted"),
        }
    }
}
impl std::error::Error for CacheError {}

pub struct Cache {
    memory: super::memory::Memory<[u64; 128]>,
    pub(crate) programs: [Program; 8],
    fill: fn(&mut [[u64; 128]]),
}

impl Cache {
    pub fn new(key: &[u8]) -> Result<Self, CacheError> {
        Self::with_initializer(key, fill_memory)
    }

    pub(crate) fn with_initializer(
        key: &[u8],
        fill: fn(&mut [[u64; 128]]),
    ) -> Result<Self, CacheError> {
        u32::try_from(key.len()).map_err(|_| CacheError::KeyTooLong)?;
        let mut memory = Vec::new();
        memory
            .try_reserve_exact(BLOCKS)
            .map_err(CacheError::Allocation)?;
        memory.resize(BLOCKS, [0; 128]);
        Self::with_memory(key, memory.into(), fill)
    }

    pub fn with_memory(
        key: &[u8],
        mut memory: super::memory::Memory<[u64; 128]>,
        fill: fn(&mut [[u64; 128]]),
    ) -> Result<Self, CacheError> {
        if memory.len() != BLOCKS {
            return Err(CacheError::InvalidRange);
        }
        let initial = initial_hash(key)?;
        first_blocks(&mut memory, initial);
        fill(&mut memory);
        let programs = programs(key);
        Ok(Self {
            memory,
            programs,
            fill,
        })
    }

    /// Reuse the 256 MiB allocation when switching keys. Invalid lengths leave
    /// the previous cache untouched; the first pass overwrites every block.
    /// Retains the originally selected Argon2 kernel.
    pub fn reinitialize(&mut self, key: &[u8]) -> Result<(), CacheError> {
        let initial = initial_hash(key)?;
        first_blocks(&mut self.memory, initial);
        (self.fill)(&mut self.memory);
        self.programs = programs(key);
        Ok(())
    }

    pub fn page_size(&self) -> usize {
        self.memory.page_size()
    }
    pub fn words(&self) -> &[u64] {
        self.memory.as_flattened()
    }

    /// Calculate a dataset item without allocating the complete 2 GiB dataset.
    pub fn dataset_item(&self, item: u64) -> [u64; 8] {
        let first = item.wrapping_add(1).wrapping_mul(6364136223846793005);
        let mut registers = ADDS.map(|value| first ^ value);
        let mut address = item;
        for program in &self.programs {
            let offset = (address as usize & (CACHE_SIZE / 64 - 1)) * 8;
            program.execute(&mut registers);
            for (register, word) in registers.iter_mut().zip(&self.words()[offset..offset + 8]) {
                *register ^= word;
            }
            address = registers[program.address_register()];
        }
        registers
    }

    /// Fill caller-owned contiguous dataset storage. Disjoint slices can be
    /// populated concurrently while sharing the immutable cache.
    pub fn fill_dataset(&self, start: usize, output: &mut [[u64; 8]]) -> Result<(), CacheError> {
        if start
            .checked_add(output.len())
            .is_none_or(|end| end > DATASET_ITEMS)
        {
            return Err(CacheError::InvalidRange);
        }
        for (i, item) in output.iter_mut().enumerate() {
            *item = self.dataset_item((start + i) as u64);
        }
        Ok(())
    }
}

fn programs(key: &[u8]) -> [Program; 8] {
    let mut generator = Blake2Generator::new(key, 0);
    std::array::from_fn(|_| scheduler::generate(&mut generator).prepare())
}

fn initial_hash(key: &[u8]) -> Result<[u8; 72], CacheError> {
    let length = u32::try_from(key.len()).map_err(|_| CacheError::KeyTooLong)?;
    let mut hash = Blake2b::new(64).unwrap();
    // Zero tag length is intentional: RandomX consumes Argon2's memory,
    // rather than its final password hash. Version 0x13, type Argon2d = 0.
    for value in [1, 0, BLOCKS as u32, PASSES, 0x13, 0, length] {
        hash.update(&value.to_le_bytes());
    }
    hash.update(key);
    hash.update(&8u32.to_le_bytes());
    hash.update(b"RandomX\x03");
    hash.update(&[0; 8]); // empty secret and associated data
    let mut initial = [0; 72];
    hash.finalize(&mut initial[..64]).unwrap();
    Ok(initial)
}

fn first_blocks(memory: &mut [[u64; 128]], mut initial: [u8; 72]) {
    for (i, block) in memory[..2].iter_mut().enumerate() {
        initial[64..68].copy_from_slice(&(i as u32).to_le_bytes());
        let mut bytes = [0; 1024];
        blake2b::long_hash(&initial, &mut bytes).unwrap();
        for (word, bytes) in block.iter_mut().zip(bytes.as_chunks::<8>().0) {
            *word = u64::from_le_bytes(*bytes);
        }
    }
}
pub fn fill_memory(memory: &mut [[u64; 128]]) {
    for pass in 0..PASSES {
        for slice in 0..4 {
            let start = if pass == 0 && slice == 0 { 2 } else { 0 };
            for index in start..SEGMENT {
                let current = slice * SEGMENT + index;
                let previous = current.wrapping_sub(1) & (BLOCKS - 1);
                let random = memory[previous][0] as u32;
                let area = if pass == 0 {
                    slice * SEGMENT + index - 1
                } else {
                    BLOCKS - SEGMENT + index - 1
                };
                let squared = (u64::from(random) * u64::from(random)) >> 32;
                let relative = area - 1 - ((area as u64 * squared) >> 32) as usize;
                let beginning = if pass == 0 { 0 } else { (slice + 1) * SEGMENT };
                let reference = (beginning + relative) & (BLOCKS - 1);
                // Argon2's reference area excludes both the current and previous
                // block. Disjoint borrows let compression write its destination
                // directly instead of returning/copying a 1 KiB temporary.
                let [previous, reference, output] = memory
                    .get_disjoint_mut([previous, reference, current])
                    .expect("Argon2 reference area excludes current and previous blocks");
                fill_block_into(previous, reference, output, pass != 0);
            }
        }
    }
}

#[inline]
fn blamka(a: u64, b: u64) -> u64 {
    a.wrapping_add(b)
        .wrapping_add(2u64.wrapping_mul(u64::from(a as u32) * u64::from(b as u32)))
}

#[inline(always)]
fn mix(block: &mut [u64; 16], [a, b, c, d]: [usize; 4]) {
    block[a] = blamka(block[a], block[b]);
    block[d] = (block[d] ^ block[a]).rotate_right(32);
    block[c] = blamka(block[c], block[d]);
    block[b] = (block[b] ^ block[c]).rotate_right(24);
    block[a] = blamka(block[a], block[b]);
    block[d] = (block[d] ^ block[a]).rotate_right(16);
    block[c] = blamka(block[c], block[d]);
    block[b] = (block[b] ^ block[c]).rotate_right(63);
}

#[inline(always)]
fn round(block: &mut [u64; 16]) {
    // Constant lane indices allow the scalar backend to retain the entire
    // permutation in registers instead of repeatedly indexing the 1 KiB block.
    mix(block, [0, 4, 8, 12]);
    mix(block, [1, 5, 9, 13]);
    mix(block, [2, 6, 10, 14]);
    mix(block, [3, 7, 11, 15]);
    mix(block, [0, 5, 10, 15]);
    mix(block, [1, 6, 11, 12]);
    mix(block, [2, 7, 8, 13]);
    mix(block, [3, 4, 9, 14]);
}

fn fill_block_into(
    previous: &[u64; 128],
    reference: &[u64; 128],
    output: &mut [u64; 128],
    with_xor: bool,
) {
    let mut block = [0; 128];
    for i in 0..128 {
        block[i] = previous[i] ^ reference[i];
        output[i] = block[i] ^ if with_xor { output[i] } else { 0 };
    }
    for row in block.as_chunks_mut::<16>().0 {
        round(row);
    }
    for i in 0..8 {
        let mut column = std::array::from_fn(|j| block[2 * i + (j / 2) * 16 + j % 2]);
        round(&mut column);
        for (j, value) in column.into_iter().enumerate() {
            block[2 * i + (j / 2) * 16 + j % 2] = value;
        }
    }
    for i in 0..128 {
        output[i] ^= block[i];
    }
}

#[cfg(all(test, any(unix, windows), target_arch = "x86_64"))]
pub(crate) fn fill_block(
    previous: &[u64; 128],
    reference: &[u64; 128],
    old: Option<&[u64; 128]>,
) -> [u64; 128] {
    let mut output = old.copied().unwrap_or([0; 128]);
    fill_block_into(previous, reference, &mut output, old.is_some());
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "initializes a 256 MiB Argon2d cache; use an optimized test build"]
    fn upstream_cache_and_dataset_vectors() {
        let cache = Cache::new(b"test key 000").unwrap();
        for (index, expected) in [
            (0, 0x191e0e1d23c02186),
            (1568413, 0xf1b62fe6210bf8b1),
            (33554431, 0x1f47f056d05cd99b),
        ] {
            assert_eq!(cache.words()[index], expected);
        }
        for (item, expected) in [
            (0, 0x680588a85ae222db),
            (10000000, 0x7943a1f6186ffb72),
            (20000000, 0x9035244d718095e1),
            (30000000, 0x145a5091f7853099),
        ] {
            assert_eq!(cache.dataset_item(item)[0], expected);
        }
        let mut last = [[0; 8]; 2];
        cache.fill_dataset(DATASET_ITEMS - 2, &mut last).unwrap();
        assert_eq!(last[1], cache.dataset_item(DATASET_ITEMS as u64 - 1));
        let before = last;
        assert!(cache.fill_dataset(DATASET_ITEMS - 1, &mut last).is_err());
        assert_eq!(last, before);
        assert!(cache.fill_dataset(usize::MAX, &mut last).is_err());
    }
}
