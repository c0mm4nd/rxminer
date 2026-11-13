//! Portable Rust reference implementation of RandomX v1 and v2.
//!
//! Includes cache/dataset construction, software AES, explicit binary64 rounding
//! and the complete interpreter/hash pipeline. The miner adds native JITs and
//! an x86-64 SIMD interpreter; this safe core remains an independent reference.
//! This module also compiles as a standalone Rust library, without build.rs,
//! native code or external crates. See docs/RANDOMX.md for the migration stages.
#![forbid(unsafe_code)]

pub mod aes;
pub mod blake2b;
pub mod cache;
pub mod float;
pub mod generator;
pub mod memory;
pub mod scheduler;
pub mod superscalar;
pub mod vm;

/// RandomX commitment: BLAKE2b-256(input || hash).
pub fn commitment(input: &[u8], hash: &[u8; 32]) -> [u8; 32] {
    let mut state = blake2b::Blake2b::new(32).unwrap();
    state.update(input);
    state.update(hash);
    let mut out = [0; 32];
    state.finalize(&mut out).unwrap();
    out
}
