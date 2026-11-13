#[cfg(all(feature = "randomx-rust", feature = "randomx-cpp"))]
compile_error!(
    "select exactly one RandomX backend; use --no-default-features --features randomx-cpp"
);
#[cfg(not(any(feature = "randomx-rust", feature = "randomx-cpp")))]
compile_error!("select a RandomX backend: randomx-rust or randomx-cpp");

pub mod config;
pub mod control;
mod counters;
mod developer;
pub mod engine;
pub mod http;
pub mod job;
pub mod miner;
pub mod proxy;
pub mod randomx;
#[cfg(feature = "randomx-rust")]
pub use randomx_rust::{randomx_core, randomx_memory};
pub mod stratum;
mod tls;
pub mod topology;

pub mod logging;
pub mod runtime;

pub mod dns;

pub mod msr;

pub mod power;

#[cfg(all(
    feature = "randomx-rust",
    any(
        all(any(unix, windows), target_arch = "x86_64"),
        all(unix, target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "riscv64")
    )
))]
pub use randomx_rust::randomx_jit;
