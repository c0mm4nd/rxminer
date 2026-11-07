# RxMiner

[![CI](https://github.com/c0mm4nd/rxminer/actions/workflows/ci.yml/badge.svg)](https://github.com/c0mm4nd/rxminer/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-blue.svg)](https://www.rust-lang.org)

RandomX (`rx/0`) CPU miner. Rust control plane (config, workers, Stratum,
HTTP API) over an integrated RandomX C/C++ hashing library with JIT backends.

- Cargo-only build; no CMake, Make or Ninja
- NUMA-local dataset replication, CPU affinity, 2 MiB huge pages, light mode
- Stratum over TCP/TLS, certificate pinning, failover, SOCKS5 / HTTP CONNECT
- JSON config with live reload; optional authenticated HTTP API
- Opt-in MSR tuning, restored on exit
- Offline benchmark mode

| Target | Backend |
| --- | --- |
| Linux x86-64 | Optimized x86-64 JIT |
| Linux ARM64 | ARM64 JIT |
| macOS x86-64 | Optimized x86-64 JIT |
| macOS ARM64 | ARM64 JIT, W^X |
| Windows x64 (MSVC) | Windows x64 JIT |

Details: [platforms](docs/PLATFORMS.md),
[implementation status](docs/CONFIGURATION.md#remaining-implementation-work).

## Build

Requires Rust 1.88+, a C/C++ toolchain and OpenSSL headers
([per-platform](docs/PLATFORMS.md#dependencies)).

```bash
sudo apt-get install -y gcc g++ pkg-config libssl-dev   # Debian/Ubuntu

cargo install --git https://github.com/c0mm4nd/rxminer.git --locked --bin rxminer
# or, from a checkout:
cargo install --path . --locked --bin rxminer
```

Host-specific build:

```bash
RXMINER_ARCH=native RUSTFLAGS='-C target-cpu=native' cargo build --release --locked
```

## Usage

```bash
rxminer -o stratum+ssl://pool.example.com:443 -u WALLET -p x --tls-verify
rxminer -c config.json --dry-run      # validate config and CPU selection only
rxminer --benchmark-seconds 60        # offline, no pool, no fee
```

`--tls` without `--tls-verify` does not verify certificates.
Start from [`config.example.json`](config.example.json); CLI flags override JSON.

Memory: ~2.3 GiB per dataset (one per NUMA node unless `--randomx-no-numa`),
plus 2 MiB scratchpad per worker. `--randomx-mode light` uses the 256 MiB
cache only, at lower hashrate.

Defaults: workers = allowed physical cores (Linux honors affinity and cgroup
quota); huge pages requested with fallback; HTTP API off, loopback and
read-only when enabled (`--http-port`); control requires a token and
`http.restricted: false`.

Full reference: [configuration](docs/CONFIGURATION.md),
[tuning](docs/OPTIMIZATION.md).

## Performance

RxMiner 0.5.0 vs XMRig 6.26.0, 2026-11-04, matched settings, mean of 2 runs:

| Run | RxMiner kH/s | XMRig kH/s | Delta |
| --- | ---: | ---: | ---: |
| A | 38.55 | 33.76 | +14.20% |
| B | 46.89 | 39.80 | +17.80% |

Historical results on shared hosts; measure on your own hardware.

## Developer fee

Uniform random rate in `[0, 1)` %, fixed per process. Each 600 s cycle opens
with the developer window on a separate connection to the same pool and seed.
Not configurable. Developer connection loss pauses mining until reconnect or
failover (fatal without a backup pool); seed mismatch is fatal. None in
benchmarks.

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

MIT. Bundled RandomX: [BSD 3-Clause](src/crypto/randomx/LICENSE).
