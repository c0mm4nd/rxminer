// SPDX-License-Identifier: MIT
//! Pure Rust RandomX v1/v2, portable interpreter and native Rust JITs.
mod backend;
#[cfg(target_arch = "aarch64")]
mod randomx_aes_arm;
pub mod randomx_core;
#[cfg(target_arch = "x86_64")]
mod randomx_interpreter;
#[cfg(all(any(unix, windows), target_arch = "x86_64"))]
pub mod randomx_jit;
#[cfg(all(unix, target_arch = "aarch64"))]
#[path = "randomx_jit/aarch64/mod.rs"]
pub mod randomx_jit;
#[cfg(all(target_os = "linux", target_arch = "riscv64"))]
#[path = "randomx_jit/riscv64/mod.rs"]
pub mod randomx_jit;
pub mod randomx_memory;
pub use backend::*;
pub use randomx_core::commitment;
pub const BACKEND: &str = "Rust";
