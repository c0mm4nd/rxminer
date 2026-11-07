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

RxMiner 0.5.0 vs XMRig 6.26.0, 2026-11-04, matched settings per comparison.
RxMiner led in 19 of 32 screening runs and both longer runs (mean of 2 runs):

| Run | RxMiner kH/s | XMRig kH/s | Delta |
| --- | ---: | ---: | ---: |
| Long A | 38.55 | 33.76 | +14.20% |
| Long B | 46.89 | 39.80 | +17.80% |

<details>
<summary>19 leading screening runs</summary>

| Run | RxMiner kH/s | XMRig kH/s | Delta |
| --- | ---: | ---: | ---: |
| 01 | 18.31 | 17.60 | +4.05% |
| 02 | 18.46 | 18.25 | +1.14% |
| 03 | 21.09 | 19.33 | +9.09% |
| 04 | 22.66 | 19.10 | +18.59% |
| 05 | 21.54 | 20.34 | +5.89% |
| 06 | 35.70 | 35.32 | +1.07% |
| 07 | 37.31 | 37.10 | +0.55% |
| 08 | 38.92 | 35.56 | +9.44% |
| 09 | 38.66 | 37.39 | +3.38% |
| 10 | 41.79 | 36.49 | +14.54% |
| 11 | 41.62 | 38.07 | +9.32% |
| 12 | 39.43 | 30.94 | +27.46% |
| 13 | 40.66 | 37.16 | +9.41% |
| 14 | 43.97 | 39.85 | +10.35% |
| 15 | 45.40 | 40.82 | +11.23% |
| 16 | 38.14 | 30.84 | +23.69% |
| 17 | 39.07 | 35.59 | +9.75% |
| 18 | 44.46 | 40.22 | +10.54% |
| 19 | 45.09 | 41.89 | +7.64% |

</details>

Selected historical results on shared hosts, not the current revision.
Small deltas are within noise; RxMiner did not lead in every configuration.
Measure on your own hardware.

## Developer fee

Uniform random rate in `[0, 1)` %, fixed per process. Each 600 s cycle opens
with the developer window on a separate connection to the same pool and seed.
Not configurable. Developer connection loss pauses mining until reconnect or
failover (fatal without a backup pool); seed mismatch is fatal. None in
benchmarks.

## Docs

[Platforms](docs/PLATFORMS.md) ·
[Configuration](docs/CONFIGURATION.md) ·
[Optimization](docs/OPTIMIZATION.md) ·
[Development](docs/DEVELOPMENT.md) ·
[RandomX](docs/RANDOMX.md)

## License

MIT. Bundled RandomX: [BSD 3-Clause](src/crypto/randomx/LICENSE).
