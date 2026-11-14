# Development

The workspace contains the miner, two selectable RandomX backends and the shared
platform crate. Select exactly one backend when building or testing the miner.

## Build and standard checks

Requires Rust 1.88+, Cargo, a linker and the C toolchain used by TLS's ring
provider. RandomX is Rust by default. The pure Rust RandomX crate has no build script or native
build dependency. The optional C++ crate builds its bundled sources using `cc`;
it needs a C++ toolchain, but neither backend requires CMake, Make or Ninja.

```bash
cargo build --release --locked
cargo fmt --check
cargo clippy -p rxminer -p randomx-rust -p rx-platform --locked --all-targets --features test-dev-fee -- -D warnings
cargo test -p rxminer -p randomx-rust -p rx-platform --locked --release --features test-dev-fee
./target/release/rxminer --benchmark-seconds 1 --randomx-mode light --threads 1 --no-huge-pages
```

Use optimized tests for the portable interpreter. Test builds with `test-dev-fee`
are local-only and must not be distributed. Release builds use default features.

## Backend crates and feature checks

`randomx-rust` and `randomx-cpp` expose the same `Dataset`, `Vm`, `Tuning`,
`MemoryMode`, pipeline and allocator-policy API. Dataset construction takes a
worker count and setup callback; only the miner layer knows about topology and
logging. The Rust crate also exposes its independent core/JIT modules for tests.
`Vm::with_tuning` shares initialized seed data across execution configurations.
C++ VM seed replacement prepares a new VM before releasing the old one, retaining
its execution settings and leaving it usable on allocation failure.

The miner features are mutually exclusive; do not use `--all-features`.
Default workspace commands select only the miner, so C++ selection does not
also build an unused Rust backend. Run backend crate tests explicitly:

```bash
cargo build --locked --release --no-default-features --features randomx-cpp
cargo clippy -p rxminer -p randomx-cpp --locked --all-targets \
  --no-default-features --features rxminer/randomx-cpp -- -D warnings
cargo test -p rxminer --locked --release --no-default-features --features randomx-cpp
cargo test -p randomx-rust --locked --release
cargo test -p randomx-cpp --locked --release
# Identical contract under both selected backends, including the 72-mode sweep:
cargo test -p rxminer --locked --release --test backend_contract -- --include-ignored --test-threads=1
cargo test -p rxminer --locked --release --no-default-features --features randomx-cpp \
  --test backend_contract -- --include-ignored --test-threads=1
```

The common contract checks v1/v2 known hashes, pipelines with version changes,
seed replacement, initialization cancellation/errors, full/light modes and
exact x86 MXCSR restoration. CPU-unsupported modes are skipped explicitly.
Only the C++ crate has a `cc` build dependency; the application's TLS provider
still has its own native dependencies in both configurations.

Local and Git installs resolve both backend packages from this workspace.
A registry release must publish the backend and platform crates before publishing the miner;
an unpublished multi-crate workspace cannot be verified as a standalone miner
`.crate` against crates.io. The tooling test packages/verifies the independent
Rust backend and platform crates and checks local/Git workspace installs without C++ tools.

The local Linux x86-64 split validation (2026-11-13) passes the 77 retained
Rust/miner library tests, the 51 shared miner tests with C++ selected, and each
backend's six contract tests including its 72-mode sweep. A fresh separate CMake
reference also passes the existing 72-mode Rust differential. C++ full-dataset
worker/fee switches and its CLI smoke pass. Both crates pass packaging verification,
Clippy and Rust 1.88 application builds; both/neither feature selections fail as
intended. The guarded Rust-only package/local/Git-install test passes without C++
build tools. Rust crate cross builds pass for Windows x64, both macOS targets,
Linux ARM64 and RV64; this does not claim native execution on those systems or
whole-application cross builds. Raw logs and fingerprints are in
`target/backend-split/`. No remote CI was run.

## Platform crate

`crates/rx-platform` owns CPU discovery, SMT/NUMA-aware worker selection,
affinity, priorities, NUMA binding, cgroup CPU limits and memory-headroom queries.
It depends only on `anyhow` and OS bindings; it needs no RandomX backend, logging
implementation or native build script. `src/topology.rs` retains the miner's
RandomX dataset replication budget and translates discovery warnings into logs.
The original `rxminer::topology` API remains available.

```bash
cargo test -p rx-platform --locked --release
cargo clippy -p rx-platform --locked --all-targets -- -D warnings
cargo package -p rx-platform --locked --allow-dirty
```

Platform tests must be selected explicitly because the default workspace member
is the miner. The CI definitions select them on Linux, Windows, macOS and RV64;
local cross compilation alone does not validate their native OS behavior.

Local extraction validation passes 78 Rust-backend/miner/platform library tests,
48 miner library tests with C++ selected and five common contract tests for each
backend. Both feature configurations pass Clippy. The platform crate passes
Rust 1.88 checks, independent packaging verification and all-target compile
checks for Windows x64, macOS Intel/ARM64, Linux ARM64 and RV64. The updated
package/local/Git-install guard test passes, including actual light-mode hashes.
These CLI hashes validate installation, not comparative performance. Logs are in
`target/platform-split/`; no remote CI or publication was requested.

## Complete Rust core checks

The standalone core has no external crate dependencies and forbids unsafe code.
It can be tested without Cargo, TLS or any C/C++ compilation:

```bash
rustc --edition=2021 -O --test crates/randomx-rust/src/randomx_core/mod.rs -o /tmp/randomx-core-tests
/tmp/randomx-core-tests
/tmp/randomx-core-tests --ignored --test-threads=1
cargo test -p randomx-rust -p rxminer --locked --release --lib -- --ignored --test-threads=1
cargo test --locked --release --test portable -- --ignored --nocapture
```

The ignored tests need at least 4 GiB free memory and several minutes. Run
expensive tests sequentially. They check upstream complete hashes, full/light
agreement, VM reuse, worker failures, seed changes and lifecycle cleanup.
On ARM64 and RV64, `randomx_jit::tests` also checks every instruction class,
random mixed programs, all eight chained programs, full/light pipelines and
replacement seeds. These tests can run on native hosts or through a Cargo
cross-target runner such as QEMU with a matching target sysroot. QEMU verifies
behavior, not native performance or privileged page allocation.

On ARM64, `randomx_aes_arm::tests::hardware_matches_software` checks fill,
program generation, fused hash/refill and v2 register mixing against software
AES when the CPU exposes AES instructions. The memory-policy test changes
process-wide allocator settings; run it with one test thread.
See [migration coverage](RANDOMX.md#rust-migration) for scope and limitations.

## Optional native differential tests

These tests require a full checkout and a separate C/C++ reference build, currently
configured for Linux x86-64. These are the same maintained sources packaged by
`randomx-cpp`; this separate shared-library build remains optional and is never
linked by the `randomx-rust` feature.

```bash
cmake -S . -B target/native-reference -DBUILD_SHARED_LIBS=ON
cmake --build target/native-reference --target randomx-oracles -j 8
RXMINER_NATIVE_LIBRARY="$PWD/target/native-reference/librandomx.so" cargo test --locked --release --test randomx_rust -- --ignored --nocapture
RXMINER_NATIVE_LIBRARY="$PWD/target/native-reference/librandomx.so" cargo test --locked --release --test randomx_modes -- --ignored --nocapture
RXMINER_ORACLE_DIR="$PWD/target/native-reference" cargo test --locked --release --test native_vectors -- --ignored --nocapture
```

The first test loads the explicitly selected reference library for BLAKE2b,
Argon2 output expansion and reciprocal comparisons. `randomx_modes` checks
72 full/light, interpreter/JIT, AES, secure, prefetch and v1/v2 combinations,
plus pipelines, source replacement and commitment. The vector suite runs prebuilt
executables to compare generation, scheduling, floating-point rounding, AES,
cache/dataset output and complete v1/v2 hashes. It also retains native JIT and
interpreter regressions. Missing reference paths fail explicitly; ordinary
`cargo test` ignores these developer-only tests.

## Protocol and runtime checks

```bash
cargo test --locked --release --features test-dev-fee --test network --target-dir target/test-dev -- --ignored --test-threads=1 --nocapture
```

These Linux x86-64 checks require `taskset`. Run them without concurrent mining
or dataset initialization: the priority-control case deliberately lowers worker
priority and can be starved by other CPU-bound tests. They use light mode, loopback Rust
pool/proxy fixtures, rcgen test certificates and rustls. They never connect to
public pools. Coverage includes TLS rejection, HTTP CONNECT/SOCKS5, shares,
nonce continuity, failover, NiceHash, keepalive, authenticated HTTPS control,
pause/resume, reload, background mode and autosave. Fixtures accept shares
without checking proof-of-work; hash correctness is covered separately.
Recovery checks occupy the HTTP listener and fix its port during retry backoff,
kill the mining child to verify the default foreground supervisor,
and stop only the parent during mining and retry delays. Unit tests inject
coordinator panics and verify monitor cleanup and bounded recovery delays.

## Packaging and Cargo installation

```bash
cargo test --locked --release --test tooling cargo_install_without_external_build_tools -- --ignored --nocapture
```

This Linux x86-64 check needs Git and `taskset`. It verifies packaging, fresh
local and temporary Git installations, and hashing by installed binaries.
C++ compilers, Python, CMake, Make, Ninja, OpenSSL and pkg-config are blocked;
C compilers remain available for ring and linking. Installed binaries must not
link OpenSSL or libstdc++. The temporary repository is never pushed.

## CI status

CI checks both backend features separately. Linux jobs also build the optional
C++ reference for Rust differential tests; platform jobs run vectors and
full/light contracts. Check the GitHub Actions result for the exact commit under
review. Historical C++ results do not validate the Rust ports or the new crate
boundaries. See [platform support](PLATFORMS.md).
