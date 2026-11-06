# RxMiner

RxMiner is a **Rust RandomX CPU miner** for Monero-style `rx/0` pools.
The CLI, configuration, CPU/NUMA selection, worker lifecycle, Stratum transport,
proxy support, developer-fee scheduling, and HTTP API are implemented in Rust.
The integrated [RandomX](src/crypto/randomx/README.md) C/C++ library supplies
the consensus hash and CPU JIT through a small, owned FFI layer in
[`src/randomx.rs`](src/randomx.rs).

Linux x86-64 is supported, with pool failover, light/full-memory mining,
configuration reload and an authenticated control API. GPU backends, other
platforms and additional algorithms remain unsupported; see the
[implementation status](docs/CONFIGURATION.md#remaining-implementation-work).

## Install with Cargo

Linux x86-64 is supported. Install Rust 1.88+ and Cargo, a C/C++ compiler,
pkg-config and OpenSSL development headers. On Debian/Ubuntu:

```bash
sudo apt-get install -y gcc g++ pkg-config libssl-dev
```

Install from this local checkout with one command; no CMake/Make/Ninja or
separate native-library configuration is needed:

```bash
cargo install --path . --locked
rxminer --version
rxminer --help
rxminer --benchmark-seconds 20
```

Git installation uses the same build. Once the project is uploaded, install
from its GitHub repository:

```bash
RXMINER_REPOSITORY="https://github.com/c0mm4nd/rxminer.git"
cargo install --git "$RXMINER_REPOSITORY" --locked --bin rxminer
rxminer --version
```

To pin an existing release, add `--tag v0.5.0`; to pin a commit, add
`--rev COMMIT`. A tag must exist in the remote before using it. Re-run the Git
install command to update to the repository's current default branch.

Cargo compiles a release binary and installs it in `~/.cargo/bin` by default;
ensure that directory is on `PATH`. RandomX's complete native source is
included; Cargo's `cc` build dependency compiles it directly. The resulting
command is the Rust miner. OpenSSL and the C++ runtime are
system libraries. Cross-compilation is not currently configured.

crates.io publication is deferred; `cargo install rxminer` is not yet a
supported distribution method. Use the Git or local-source command above.

## Build

Requirements: Rust 1.88+, Cargo, a C/C++ compiler, pkg-config,
and OpenSSL development headers (`libssl-dev`).

```bash
cargo build --release --locked
./target/release/rxminer --version
```

Cargo's `build.rs` uses `cc` to compile the integrated RandomX native library
without an external build system. `Cargo.lock` pins Rust dependencies;
the first build may download crates. After `cargo fetch --locked`, use Cargo's
`--offline` option for offline builds.

Portable builds are the default: RandomX uses `ARCH=default` and selects JIT/AES
features at runtime. For a build used only on the build host:

```bash
RXMINER_ARCH=native RUSTFLAGS='-C target-cpu=native' cargo build --release --locked
```

Do not redistribute a native build. The release workflow builds with the
portable defaults and without the `test-dev-fee` feature.

## Usage

```bash
./target/release/rxminer \
  -o stratum+ssl://pool.example.com:443 -u YOUR_WALLET -p x \
  --tls-verify --http-port 2333

# Local full-memory benchmark; no network or pool connection
./target/release/rxminer --benchmark-seconds 20

# JSON configuration; CLI values override file values
cp config.example.json config.json
./target/release/rxminer -c config.json
```

Configuration uses `pools`, `cpu`, `randomx`, and `http` sections. Common option
names follow the [configuration reference](docs/CONFIGURATION.md); developer
fees remain fixed. CLI values override JSON values. `--dry-run` validates the
configuration and CPU selection without connecting or allocating datasets.

Automatic selection uses allowed physical cores and the cgroup CPU quota.
`--smt` permits additional logical CPUs. `-t N` requests exactly N workers;
use `--cpu-affinity MASK` or a `cpu.rx` profile to bind them to specific CPUs.
`--randomx-init N` limits initialization threads per dataset within the allowed
CPU set and quota. `--randomx-no-numa` selects one shared dataset.

Explicit 2 MiB huge pages are enabled by default (`cpu.huge-pages: true`);
`--no-huge-pages` disables them. Allocation falls back when reserved pages are
unavailable. Optional 1 GiB dataset pages and JIT huge pages use existing system
reservations. MSR changes are opt-in and restored on exit by default.
`--no-thp` separately disables transparent huge-page hints on fallback memory.
Run `--help` for supported CLI options.

Explicit allocation uses checked writable prefaulting (Linux 5.14+). The
startup log and `/summary` report actual huge-page dataset and scratchpad
counts; requesting the flag does not imply full coverage. Each dataset needs
1,040 reserved 2 MiB pages and each worker needs one. With NUMA replication,
reserve enough pages on each worker node. The temporary initialization cache
uses ordinary/THP memory, so it does not consume the explicit pool. The miner
still requires permission to allocate from that pool. Optional 1 GiB dataset
mappings require three reserved pages per dataset.

CPU discovery respects online CPU IDs and process affinity, including masks
that expose only a core's second hardware thread. Memory detection takes the
minimum of host `MemAvailable` and visible cgroup memory headroom, walking
inherited cgroup limits. Exhausted memory is distinct from an unknown limit.
If multiple NUMA datasets exceed the estimate, one dataset is selected instead.
A single full-memory dataset still needs roughly 2.3 GiB during initialization,
plus worker scratchpads and runtime overhead.

## TLS and proxies

Use `-o pool.example.com:443 --tls` or `pools[].tls: true` for TLS.
URL schemes `stratum+ssl://` / `stratum+tls://` also enable TLS;
`stratum+tcp://` / `stratum://` select plaintext TCP. The default port is 3333.

TLS is encrypted but certificate verification is off by default for compatibility
with common pool configurations. Use `--tls-verify` for the system CA store,
`--tls-ca FILE` for a private CA (also enables verification), or
`--tls-fingerprint HEX` to pin the certificate's SHA-256 digest independently
of CA trust. TLS 1.2 is the minimum version.

```bash
./target/release/rxminer -o stratum+ssl://pool.example.com:443 -u WALLET \
  --proxy http://127.0.0.1:7890

./target/release/rxminer -o stratum+ssl://pool.example.com:443 -u WALLET \
  --proxy socks5h://user:pass@127.0.0.1:1080
```

A bare `-x HOST:PORT` selects SOCKS5 with remote DNS. HTTP CONNECT and SOCKS5
authentication are supported. `socks5://` resolves pool
names locally; `socks5h://` resolves them through the proxy. TLS runs end-to-end
through the tunnel. TLS-to-proxy (`https://` proxy URLs) is not supported.

Without an explicit proxy, `HTTPS_PROXY` / `ALL_PROXY` and lowercase variants
are honored, subject to `NO_PROXY`. `--no-proxy` ignores environment proxy
settings. Both user and developer connections use the same proxy configuration.

## Built-in developer fee

The existing fee policy is preserved: a process-wide rate uniformly selected
from `[0, 1)` percent, displayed as `<1%`, with a 600-second cycle. Each cycle
starts with the developer window, followed by the user's remaining time.
There are no CLI, configuration, or environment overrides for the rate,
address, cycle, or destination.

The fixed developer address is:

```text
46fca3jxDH5RxoRkAg2DcCPYWyMGFLYQgRZrYTR7qyd2GLqLq8hvSiMSRQ5YWcZ5QBfYB1nCBb2J2aiww7bP7YHC21wZecJ
```

A second connection logs into the configured pool with this address. Its job
must use the same RandomX seed as the active user work. Developer login,
connection, seed, or job failures stop mining. Connection failures can switch
to a configured backup pool; without a backup they terminate the miner with a
nonzero status. Seed mismatches remain fatal.
Workers resume their nonce positions when returning to unchanged jobs across
fee windows, avoiding duplicate shares. Benchmark mode has no pool connection
and no fee.

For deterministic local regression tests only:

```bash
cargo build --release --locked --features test-dev-fee --target-dir target/test-dev
```

That binary uses 0.9% over a five-second cycle. It must not be packaged as a
release. The default build contains no runtime way to enable this behavior.

## HTTP API

`--http-port PORT` enables the API (read-only by default, disabled until enabled), bound to
`127.0.0.1` unless `--http-host` is specified:

- `/summary`: hashrates, shares, uptime, threads, datasets, pool connection state,
  and developer-fee phase/address.
- `/hashrates`: 10s, 60s, 15m, and lifetime rates.
- `/shares`: separate user and developer submitted/accepted/rejected counters.
- `/`: service description.

Requests are bounded and timed out so an idle HTTP client cannot prevent clean
shutdown. Set `--http-access-token TOKEN` or `http.access-token` to require
`Authorization: Bearer TOKEN` on every endpoint. Setting `http.restricted: false`
requires a token and enables pause/resume/stop and configuration updates. Root
`tls` settings can secure incoming API connections. See the
[API configuration](docs/CONFIGURATION.md#http-api) for endpoints and limits.
Initialization is visible with zero active workers until VMs are ready.

## Design

- One cache/dataset per selected NUMA node with explicit CPU affinity and
  best-effort memory binding; allocation failure can fall back to one dataset.
- Large-allocation alignment and THP hints; ordinary page alignment for small VM/JIT objects.
- JIT-assisted dataset initialization, with interpreter fallback; caches are
  released before full-memory mining begins and retained in light mode.
- Thread-local VMs hold shared dataset ownership. Seed changes join all workers
  before replacing datasets. Native pointers are confined to `randomx.rs`.
- Fixed double buffers, pipelined `first/next/last` hashing, and one job check
  per eight-hash batch. No blob allocation in the hash loop.
- Bounded submit and pending-response queues. Connection generations prevent
  old shares from being submitted under a new Stratum login.
- The main Rust scheduler serializes job installation and fee changes;
  network and HTTP threads observe shared state rather than mutating the engine.

See [`docs/OPTIMIZATION.md`](docs/OPTIMIZATION.md) for configuration and
measurement, and [`docs/RANDOMX.md`](docs/RANDOMX.md) for algorithm scope.

## Tests

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked

# Verify packaging and fresh local/Git installs with CMake/Make/Ninja blocked
# (Python 3.12+, Git, native compilers and 4 GiB free RAM; no remote writes)
python3 scripts/check_cargo_install.py --git

# Full dataset checks, seed/task switches and injected worker failures (4 GiB RAM)
cargo test --locked --release -- --ignored --test-threads=1

# Local TLS, proxies, shares/API, certificate rejection and failure handling
cargo build --release --locked --features test-dev-fee --target-dir target/test-dev
python3 tests/integration.py
python3 tests/runtime.py --binary target/test-dev/release/rxminer

# Upstream RandomX suite linked against Cargo's exact native library
python3 scripts/check_randomx.py
```

The Python pool accepts protocol-valid shares without validating proof-of-work.
The Rust full-dataset test independently checks actual worker hashes against a
light VM, while the FFI unit test checks an upstream known-answer vector.
All integration traffic stays on loopback. CI runs these checks sequentially
to avoid allocating several datasets simultaneously.

Workers cache immutable task snapshots and batch hash counts into an atomic
counter isolated from lifecycle metadata. Counts survive seed rebuilds and are flushed during
normal exit or Rust panic unwinding. Repeated notifications of identical work
preserve nonce progress. Startup failures roll back the worker set and datasets;
worker/network/API thread panics notify the coordinator instead of silently
reducing the running service. Closing a pool connection also shuts down its
active socket to interrupt TLS/proxy I/O. See the concurrency limits and
validation details in [`docs/OPTIMIZATION.md`](docs/OPTIMIZATION.md).

## Local benchmark

```bash
./target/release/rxminer --benchmark-seconds 60
```

Benchmark mode runs locally without a pool connection. Repeat measurements
with the same CPU affinity, thread count and page policy. Check actual memory
backing and allow for initialization and warmup. Performance depends on the
machine and workload; no fixed speedup is promised. See the
[performance guide](docs/OPTIMIZATION.md) for details.

## Layout and license

```text
Cargo.toml / Cargo.lock       Rust package and pinned dependencies
build.rs                     build/link the integrated RandomX library
scripts/check_cargo_install.py fresh local/Git installation without CMake/Make/Ninja
src/*.rs                     Rust miner and FFI ownership boundary
tests/randomx/               native consensus and differential regression tests
tests/integration.py          local end-to-end regression driver
tests/tls_pool.py             local TLS/TCP Stratum pool
tests/tls_proxy.py            local HTTP CONNECT / SOCKS5 proxy
src/crypto/randomx/          native hash/JIT implementation for Linux x86-64
cmake/randomx.cmake         optional native developer build configuration
CMakeLists.txt                optional native tests and benchmark probe
bench/bench_raw.cpp          optional native NUMA benchmark probe
```

RxMiner is MIT licensed. RandomX is BSD 3-Clause; its license is included in
release packages. Rust dependencies retain their respective licenses, as
identified in their Cargo package metadata.
