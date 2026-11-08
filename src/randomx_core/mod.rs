//! Portable Rust foundations for the RandomX migration.
//!
//! This is not yet a complete hash backend. The miner continues to use the
//! native VM while each component is checked against the pinned implementation.
//! This module also compiles as a standalone Rust library, without build.rs,
//! native code or external crates. See docs/RANDOMX.md for the migration stages.
#![forbid(unsafe_code)]

pub mod blake2b;
pub mod generator;
pub mod superscalar;
