<div align="center">

# ⛏️ RxMiner

**A fast, Rust-powered RandomX (`rx/0`) CPU miner for Monero-style pools.**

[![CI](https://github.com/c0mm4nd/rxminer/actions/workflows/ci.yml/badge.svg)](https://github.com/c0mm4nd/rxminer/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg?logo=rust)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.5.0-green.svg)](Cargo.toml)
<br>
![Linux](https://img.shields.io/badge/Linux-x86--64%20%7C%20ARM64-FCC624?logo=linux&logoColor=black)
![macOS](https://img.shields.io/badge/macOS-Intel%20%7C%20Apple%20Silicon-000000?logo=apple&logoColor=white)
![Windows](https://img.shields.io/badge/Windows-x64%20MSVC-0078D6?logo=windows&logoColor=white)

[Installation](#-installation) •
[Quick start](#-quick-start) •
[Performance](#-performance-vs-xmrig) •
[Configuration](#%EF%B8%8F-configuration) •
[Documentation](#-documentation)

</div>

---

RxMiner manages configuration, workers, networking and the control API in Rust,
with an integrated RandomX C/C++ library for hashing and JIT execution.

## ✨ Features

| | |
| --- | --- |
| 📦 **Cargo-only build** | Install directly with Cargo — no CMake, Make or Ninja required |
| 🧠 **CPU & memory control** | CPU affinity, NUMA placement, full-memory and light modes, huge pages |
| 🌐 **Robust networking** | Stratum over TCP or TLS, pool failover, HTTP CONNECT / SOCKS5 proxies |
| 🔧 **Flexible configuration** | JSON config with automatic reload; CLI options override JSON |
| 📡 **HTTP API** | Optional, authenticated, loopback-only and read-only by default |
| 📊 **Offline benchmarks** | Measure hashrate without pool connections or fees |
| 💸 **Low developer fee** | Built-in fee below 1% — see [details](#-developer-fee) |

### Supported platforms

| Platform | Architectures |
| --- | --- |
| 🐧 Linux | x86-64, ARM64 |
| 🍎 macOS | Intel, Apple Silicon |
| 🪟 Windows | x64 (MSVC) |

See [platform support](docs/PLATFORMS.md) for setup and system features, and
[implementation status](docs/CONFIGURATION.md#remaining-implementation-work)
for remaining features.

## 📥 Installation

### Requirements

- **Rust 1.88** or newer, with Cargo
- A native **C/C++ toolchain** and **OpenSSL** development libraries —
  see [platform dependencies](docs/PLATFORMS.md#dependencies)

On Debian or Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y gcc g++ pkg-config libssl-dev
```

### From GitHub

```bash
cargo install --git https://github.com/c0mm4nd/rxminer.git --locked --bin rxminer
rxminer --version
```

Cargo installs the binary to `~/.cargo/bin` by default; make sure that
directory is on your `PATH`. Re-run the command to update, or add
`--rev COMMIT` to install a specific revision.

### From a local checkout

```bash
cargo install --path . --locked --bin rxminer
```

> [!NOTE]
> Both methods build RandomX automatically through Cargo. OpenSSL and the C++
> runtime remain system dependencies. Installation from crates.io is not yet
> supported.

## 🚀 Quick start

### 1. Connect to a pool

Replace the example pool and wallet with your own values:

```bash
rxminer -o stratum+ssl://pool.example.com:443 \
  -u YOUR_WALLET_ADDRESS -p x --tls-verify
```

> [!IMPORTANT]
> TLS alone does **not** verify certificates. `--tls-verify` enables
> verification against the system CA store. For private CAs, certificate pins
> and proxies, see the [networking reference](docs/CONFIGURATION.md#pools-and-networking).

### 2. Use a configuration file

Copy [`config.example.json`](config.example.json), then edit the pool URL and
wallet:

```bash
cp config.example.json config.json
rxminer -c config.json --tls-verify --dry-run   # validate only
rxminer -c config.json --tls-verify             # start mining
```

`--dry-run` validates configuration and CPU selection without connecting to
pools or allocating datasets. Configuration file watching is enabled by default.

### 3. Run a local benchmark

```bash
rxminer --benchmark-seconds 60
```

Benchmarks run without pool connections or developer fees.

> [!TIP]
> Full-memory mode needs roughly **2.3 GiB per dataset**, plus worker
> scratchpads and runtime overhead. On memory-constrained machines, use
> `--randomx-mode light` — it reduces memory use at the cost of hashing speed.

## 📈 Performance vs XMRig

In historical tests comparing **RxMiner 0.5.0** with **XMRig 6.26.0**
(November 4, 2026, matching requested settings within each comparison),
RxMiner recorded higher throughput in 19 of 32 screening comparisons and in
both longer paired comparisons:

| Longer comparison | RxMiner | XMRig | RxMiner lead |
| --- | ---: | ---: | ---: |
| A | **38.55** kH/s | 33.76 kH/s | **+14.20%** |
| B | **46.89** kH/s | 39.80 kH/s | **+17.80%** |

The longer comparisons report the mean of two runs per miner.

<details>
<summary><b>Show all 19 higher-throughput screening results</b></summary>

<br>

Rates are in **kH/s**.

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

</details>

> [!NOTE]
> Percentages use unrounded measurements. These are selected historical
> results, not a benchmark of the current revision; hardware and configuration
> details are omitted. Shared-host variation affected measurements, small leads
> do not establish a repeatable advantage, and RxMiner did not lead in every
> tested configuration. Measure on your own hardware with `--benchmark-seconds`.

## ⚙️ Configuration

Run `rxminer --help` for CLI options, or see the
[configuration reference](docs/CONFIGURATION.md) for all supported settings.

| Area | Common settings | Reference |
| --- | --- | --- |
| 🌐 Pools | URL, wallet, TLS, proxies, failover | [Networking](docs/CONFIGURATION.md#pools-and-networking) |
| 🧠 CPU and memory | Threads, affinity, NUMA, huge pages, memory mode | [CPU and memory](docs/CONFIGURATION.md#cpu-and-memory) |
| 🔩 CPU tuning | Optional MSR settings and restoration | [MSRs](docs/CONFIGURATION.md#msrs) |
| 🔄 Runtime | Configuration reload, logging, background mode | [Runtime and output](docs/CONFIGURATION.md#runtime-and-output) |
| 📡 HTTP API | Status, authentication, TLS, pause/resume, configuration updates | [HTTP API](docs/CONFIGURATION.md#http-api) |
| 📊 Benchmarks | Duration, hash count, seed, checksum | [Offline benchmarks](docs/CONFIGURATION.md#offline-benchmarks) |

**Sensible defaults:**

- **Workers** — on Linux, automatic worker selection respects CPU affinity and
  cgroup CPU quotas.
- **Huge pages** — reserved 2 MiB huge pages are requested by default, with
  fallback when unavailable; RxMiner does not reserve system pages.
- **MSRs** — writes are opt-in and restored on exit by default.
- **HTTP API** — disabled by default. Enable it with `--http-port PORT`; it
  binds to loopback and starts in read-only mode. Control operations require
  an access token and `http.restricted: false`.

See the [performance guide](docs/OPTIMIZATION.md) for tuning and measurement.

## 💸 Developer fee

RxMiner selects a process-wide developer fee uniformly from `[0, 1)` percent.
Each 600-second cycle starts with the developer window, followed by user
mining. The rate, destination and schedule have no runtime overrides.

Developer work uses a separate connection to the configured pool and the same
RandomX seed as user work. A developer connection failure stops mining while
reconnecting or switching to a backup pool; without a backup, the process
exits with an error. A seed mismatch is fatal. Offline benchmarks have no fee.

## 📚 Documentation

| Document | Contents |
| --- | --- |
| [Platform support](docs/PLATFORMS.md) | Supported targets, dependencies, OS-specific features |
| [Configuration reference](docs/CONFIGURATION.md) | Every CLI flag and JSON option |
| [Performance and validation](docs/OPTIMIZATION.md) | Builds, tuning and measurement |
| [Development](docs/DEVELOPMENT.md) | Building from source and running the test suites |
| [RandomX](docs/RANDOMX.md) | RandomX support and source attribution |

## 📄 License

RxMiner is licensed under the [MIT License](LICENSE). The integrated RandomX
library is licensed under [BSD 3-Clause](src/crypto/randomx/LICENSE).
Other dependencies retain their respective licenses.
