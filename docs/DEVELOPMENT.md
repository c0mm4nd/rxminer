# Development

See [platform support](PLATFORMS.md) for verified targets and remaining differences.
Development requires Rust 1.88 or newer, Cargo,
a native C/C++ toolchain and OpenSSL development libraries; see the
platform-specific dependencies above. The native
RandomX implementation is built by Cargo; CMake, Make, Ninja and Python are
not required for the build or regression suite.

## Build

```bash
cargo build --release --locked
./target/release/rxminer --version
```

Builds use portable CPU settings by default. See the
[performance guide](OPTIMIZATION.md#build) for host-specific builds and optional
VAES support. Do not distribute binaries built with `test-dev-fee` enabled.
That feature uses a deterministic 0.9% fee over five-second cycles for tests.

## Standard checks

```bash
cargo fmt --check
cargo clippy --locked --all-targets --features test-dev-fee -- -D warnings
cargo test --locked
```

## Full dataset and native checks

The complete native differential suite runs on Linux x86-64. Allow at least
4 GiB of free memory and run these sequentially:

```bash
cargo test --locked --release --lib -- --ignored --test-threads=1
cargo test --locked --release --test tooling native_regressions -- --ignored --nocapture
```

The Rust tests cover full-memory/light-VM hash agreement, worker failures,
seed changes and lifecycle cleanup. The native suite links the exact RandomX
archive produced by the current Cargo build. It exercises upstream vectors,
JIT output, normalized program generation and BLAKE2b edge cases, including
the separate normalized JIT path. Compiler selection honors `CC` and `CXX`.

## Protocol and runtime checks

```bash
cargo test --locked --release --features test-dev-fee --test network --target-dir target/test-dev -- --ignored --test-threads=1 --nocapture
```

These Linux x86-64 tests require `taskset` (from `util-linux`) and at least 4 GiB of free
memory. They use Rust fixtures bound to loopback, generate temporary TLS
certificates through the OpenSSL library, and never connect to public pools.

Coverage includes TLS and certificate rejection, HTTP CONNECT and SOCKS5,
user/developer shares, nonce continuity, fatal developer disconnection, pool
failover, NiceHash nonce prefixes, keepalive, logging, authenticated HTTPS
control, pause/resume, configuration reload, background execution and autosave.
Fixture sockets are closed and threads are joined during teardown. Miner
subprocesses have deadlines and failure logs are retained in test output.

The fixture pool accepts protocol-valid shares without checking proof-of-work.
Hash correctness is checked independently by the Rust and native suites.

## Packaging and Cargo installation

```bash
cargo test --locked --release --test tooling cargo_install_without_external_build_tools -- --ignored --nocapture
```

This Linux x86-64 check also requires Git and `taskset`. It verifies `cargo package`, fresh
local-source and temporary Git installations, and actual hashing by each
installed binary. Each build uses a clean target directory, two compilation
jobs and portable defaults. Python, CMake, Make and Ninja are blocked for
child build processes. The temporary Git repository contains the packaged
source file set and is never pushed to a remote.

## CI

The CI workflow runs GCC with Rust 1.88 on Ubuntu 22.04 and Clang with stable
Rust on Ubuntu 24.04. Both jobs run correctness and protocol checks; the stable
job additionally verifies packaging and installation. Expensive tests run
sequentially to limit dataset memory use. Release builds use default features;
the accelerated developer-fee test binary has a separate target directory.

Cross-platform runners additionally execute:

```bash
cargo test --locked --release --lib jit_and_secure_jit_match_interpreter -- --ignored
cargo test --locked --release --lib full_dataset_shares_match_light_vm_across_fee_switches -- --ignored
cargo test --locked --release --lib worker_failures_stop_all_workers_and_clear_state -- --ignored
cargo test --locked --release --test portable -- --ignored
```

The portable tests verify CLI behavior and actual light/full-memory hashing.
The Linux-only native/protocol tools are explicitly gated rather than being
reported as tested on other platforms.
