# RxMiner

[![CI](https://github.com/c0mm4nd/rxminer/actions/workflows/ci.yml/badge.svg)](https://github.com/c0mm4nd/rxminer/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-blue.svg)](https://www.rust-lang.org)

AI-optimized RandomX miner.

Supports `rx/0` pools with selectable pure Rust and bundled C++ hashing
backends, and Rust configuration, workers, Stratum and HTTP API.

Rust JITs support x86-64, ARM64 and Linux RV64, with a selectable interpreter.
Hardware AES, x86-64 SSSE3/AVX2 cache kernels, page preferences, Linux THP policy
and scratchpad pooling are implemented. See platform notes for native, QEMU
and compile-only validation coverage.

- Cargo features select independent `randomx-rust` (default) or `randomx-cpp` crates
- Rust x86-64/ARM64/RV64 JITs for full/light v1/v2 and parallel dataset initialization
- Full/light memory modes, NUMA placement and CPU affinity
- Stratum over TCP/TLS, certificate pinning, failover, SOCKS5 / HTTP CONNECT
- JSON config with live reload; optional authenticated HTTP API
- Opt-in MSR tuning, restored on exit; offline benchmark mode

Details: [platforms](docs/PLATFORMS.md), [RandomX status](docs/RANDOMX.md).

## Build

Requires Rust 1.88+. The default `randomx-rust` backend needs no RandomX
C/C++ compiler. TLS uses ring's C/assembly kernels, so building the miner still
needs a C toolchain and linker. The `randomx-cpp` backend additionally needs a
C++ compiler (GCC/Clang, or Visual Studio C++ build tools on Windows).

```bash
# Pure Rust RandomX (default)
cargo build --release --locked
cargo install --path . --locked --bin rxminer

# Bundled C++ RandomX
cargo build --release --locked --no-default-features --features randomx-cpp
cargo install --path . --locked --no-default-features --features randomx-cpp
```

Select exactly one backend. `--features randomx-cpp` alone keeps the default
Rust feature enabled and is rejected, as is selecting neither backend.
CLI and configuration options are shared; this is a build-time choice.
Both crates are maintained in `crates/` with no build-time source download.
The Rust crate retains its JITs and portable interpreter; the C++ crate owns
its native cache, dataset and VM through a safe Rust interface.
The shared [`rx-platform`](crates/rx-platform) crate handles CPU topology, affinity,
NUMA binding and OS resource limits independently of either algorithm backend.

For a binary used only on the build machine, add `RUSTFLAGS='-C target-cpu=native'` to the build.
Backend-specific development and testing commands are in
[the development guide](docs/DEVELOPMENT.md).

Release archives identify the backend with `-rust` or `-cpp` in their filenames.
Both variants use the same CLI and configuration. See the
[release matrix](docs/DEVELOPMENT.md#release-variants) for platforms and validation.

## Usage

```bash
rxminer -o stratum+ssl://pool.example.com:443 -u WALLET -p x --tls-verify
rxminer -c config.json --dry-run      # validate config and CPU selection only
rxminer --benchmark-seconds 60        # offline, no pool connection
```

`--tls` without `--tls-verify` does not verify certificates.
Start from [`config.example.json`](config.example.json); CLI flags override JSON.

Memory: ~2.3 GiB per dataset (one per NUMA node unless `--randomx-no-numa`),
plus 2 MiB scratchpad per worker. `--randomx-mode light` uses the 256 MiB
cache only, at lower hashrate.

Defaults: workers = allowed physical cores (Linux honors affinity and cgroup
quota); reserved huge pages preferred with ordinary-page fallback; HTTP API off, loopback and
read-only when enabled (`--http-port`); control requires a token and
`http.restricted: false`.

Full reference: [configuration](docs/CONFIGURATION.md),
[tuning](docs/OPTIMIZATION.md).

## Donation

XMR:

```
46fca3jxDH5RxoRkAg2DcCPYWyMGFLYQgRZrYTR7qyd2GLqLq8hvSiMSRQ5YWcZ5QBfYB1nCBb2J2aiww7bP7YHC21wZecJ
```

## Docs

[Platforms](docs/PLATFORMS.md) ·
[Configuration](docs/CONFIGURATION.md) ·
[Optimization](docs/OPTIMIZATION.md) ·
[Development](docs/DEVELOPMENT.md) ·
[RandomX](docs/RANDOMX.md)

## License

MIT. Bundled RandomX: [BSD 3-Clause](crates/randomx-rust/src/randomx_core/LICENSE).
