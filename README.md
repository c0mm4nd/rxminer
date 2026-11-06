# RxMiner

[![CI](https://github.com/c0mm4nd/rxminer/actions/workflows/ci.yml/badge.svg)](https://github.com/c0mm4nd/rxminer/actions/workflows/ci.yml)

A Rust CPU miner for Monero-style RandomX (`rx/0`) pools on Linux x86-64.
RxMiner manages configuration, workers, networking and the control API in Rust,
with an integrated RandomX C/C++ library for hashing and JIT execution.

- Install directly with Cargo, without CMake, Make or Ninja.
- CPU affinity, NUMA placement, full-memory and light modes, and huge-page support.
- Stratum over TCP or TLS, pool failover, and HTTP CONNECT / SOCKS5 proxies.
- JSON configuration, automatic reload, and an optional authenticated HTTP API.
- Offline benchmarks and a built-in developer fee below 1%.

Currently supports Linux x86-64 and `rx/0` only. See the
[implementation status](docs/CONFIGURATION.md#remaining-implementation-work)
for unsupported platforms and features.

## Installation

### Requirements

- Rust 1.88 or newer and Cargo.
- A C/C++ compiler, `pkg-config`, and OpenSSL development headers.

On Debian or Ubuntu, install the native build dependencies with:

```bash
sudo apt-get update
sudo apt-get install -y gcc g++ pkg-config libssl-dev
```

### From GitHub

```bash
cargo install --git https://github.com/c0mm4nd/rxminer.git --locked --bin rxminer
rxminer --version
```

Cargo installs the binary to `~/.cargo/bin` by default. Ensure that directory
is on your `PATH`. Re-run the install command to update, or add `--rev COMMIT`
to install a specific revision.

### From a local checkout

Run from the repository root:

```bash
cargo install --path . --locked --bin rxminer
```

Both methods build RandomX automatically through Cargo. OpenSSL and the C++
runtime remain system dependencies. Installation from crates.io is not yet
supported.

## Quick start

### Connect to a pool

Replace the example pool and wallet with your own values:

```bash
rxminer -o stratum+ssl://pool.example.com:443 \
  -u YOUR_WALLET_ADDRESS -p x --tls-verify
```

`--tls-verify` enables certificate verification using the system CA store.
TLS alone does not enable certificate verification by default. For private
CAs, certificate pins and proxies, see the
[networking reference](docs/CONFIGURATION.md#pools-and-networking).

### Use a configuration file

Copy [config.example.json](config.example.json), then edit the pool URL and
wallet before starting:

```bash
cp config.example.json config.json
rxminer -c config.json --tls-verify --dry-run
rxminer -c config.json --tls-verify
```

CLI options override JSON values. `--dry-run` validates configuration and CPU
selection without connecting to pools or allocating datasets. Configuration
file watching is enabled by default.

### Run a local benchmark

```bash
rxminer --benchmark-seconds 60
```

Benchmarks run without pool connections or developer fees. Full-memory
initialization needs roughly 2.3 GiB per dataset, plus worker scratchpads and
runtime overhead. For memory-constrained environments, use
`--randomx-mode light`; this reduces memory use at the cost of hashing speed.

## Performance vs XMRig

RxMiner recorded higher throughput than XMRig in the historical tests below.
These results compare **RxMiner Rust 0.5.0** with **XMRig 6.26.0**, measured on
November 4, 2026, with matching requested settings within each comparison.
Hardware and configuration details are omitted here.

The table includes all 19 higher-throughput results from 32 completed screening
comparisons, followed by two longer paired comparisons. Rates are in **kH/s**;
the longer comparisons report the mean of two runs per miner.

| Test | RxMiner | XMRig | RxMiner lead |
| --- | ---: | ---: | ---: |
| Screening 01 | 18.31 | 17.60 | +4.05% |
| Screening 02 | 18.46 | 18.25 | +1.14% |
| Screening 03 | 21.09 | 19.33 | +9.09% |
| Screening 04 | 22.66 | 19.10 | +18.59% |
| Screening 05 | 21.54 | 20.34 | +5.89% |
| Screening 06 | 35.70 | 35.32 | +1.07% |
| Screening 07 | 37.31 | 37.10 | +0.55% |
| Screening 08 | 38.92 | 35.56 | +9.44% |
| Screening 09 | 38.66 | 37.39 | +3.38% |
| Screening 10 | 41.79 | 36.49 | +14.54% |
| Screening 11 | 41.62 | 38.07 | +9.32% |
| Screening 12 | 39.43 | 30.94 | +27.46% |
| Screening 13 | 40.66 | 37.16 | +9.41% |
| Screening 14 | 43.97 | 39.85 | +10.35% |
| Screening 15 | 45.40 | 40.82 | +11.23% |
| Screening 16 | 38.14 | 30.84 | +23.69% |
| Screening 17 | 39.07 | 35.59 | +9.75% |
| Screening 18 | 44.46 | 40.22 | +10.54% |
| Screening 19 | 45.09 | 41.89 | +7.64% |
| Longer comparison A | 38.55 | 33.76 | +14.20% |
| Longer comparison B | 46.89 | 39.80 | +17.80% |

Percentages use unrounded measurements. These are selected historical results,
not a new benchmark of the current revision. Shared-host variation affected
measurements; small leads do not establish a repeatable advantage, and RxMiner
did not lead in every tested configuration.

## Configuration

Run `rxminer --help` for CLI options, or see the
[configuration reference](docs/CONFIGURATION.md) for all supported settings.

| Area | Common settings | Reference |
| --- | --- | --- |
| Pools | URL, wallet, TLS, proxies, failover | [Networking](docs/CONFIGURATION.md#pools-and-networking) |
| CPU and memory | Threads, affinity, NUMA, huge pages, memory mode | [CPU and memory](docs/CONFIGURATION.md#cpu-and-memory) |
| CPU tuning | Optional MSR settings and restoration | [MSRs](docs/CONFIGURATION.md#msrs) |
| Runtime | Configuration reload, logging, background mode | [Runtime and output](docs/CONFIGURATION.md#runtime-and-output) |
| HTTP API | Status, authentication, TLS, pause/resume, configuration updates | [HTTP API](docs/CONFIGURATION.md#http-api) |
| Benchmarks | Duration, hash count, seed, checksum | [Offline benchmarks](docs/CONFIGURATION.md#offline-benchmarks) |

Automatic worker selection respects CPU affinity and cgroup CPU quotas.
Reserved 2 MiB huge pages are requested by default, with fallback when
unavailable; RxMiner does not reserve system pages. MSR writes are opt-in and
restored on exit by default. See the
[performance guide](docs/OPTIMIZATION.md) for tuning and measurement.

The HTTP API is disabled by default. Enable it with `--http-port PORT`; it
binds to loopback and starts in read-only mode. Control operations require an
access token and `http.restricted: false`.

## Developer fee

RxMiner selects a process-wide developer fee uniformly from `[0, 1)` percent.
Each 600-second cycle starts with the developer window, followed by user
mining. The rate, destination and schedule have no runtime overrides.

Developer work uses a separate connection to the configured pool and the same
RandomX seed as user work. A developer connection failure stops mining while
reconnecting or switching to a backup pool; without a backup, the process
exits with an error. A seed mismatch is fatal. Offline benchmarks have no fee.

## Documentation

- [Configuration reference](docs/CONFIGURATION.md)
- [Performance and validation](docs/OPTIMIZATION.md)
- [RandomX support and source attribution](docs/RANDOMX.md)

## License

RxMiner is licensed under the [MIT License](LICENSE). The integrated RandomX
library is licensed under [BSD 3-Clause](src/crypto/randomx/LICENSE).
Other dependencies retain their respective licenses.
