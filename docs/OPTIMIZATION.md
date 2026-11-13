# Tuning

Both RandomX backends expose the same miner configuration. Choose a backend
using measurements on your own hardware and intended workload. The project does
not recommend a backend based on bare-metal or VPS deployment.

## Build and compare locally

Build each backend separately and keep both binaries:

```bash
cargo build --release --locked
cp target/release/rxminer target/rxminer-rust
cargo build --release --locked --no-default-features --features randomx-cpp
cp target/release/rxminer target/rxminer-cpp
```

The built-in offline mode lets you compare without connecting to a pool:

```bash
target/rxminer-rust --benchmark-seconds 60 -t 1
target/rxminer-cpp --benchmark-seconds 60 -t 1
```

Stop existing miners before comparing. Use identical configuration, seed,
thread count and CPU placement, and check the actual memory mode and page
allocation reported by each run. Repeat in both orders. Measure sustained
hashrate and memory use separately; startup and dataset initialization also
matter for short-lived workloads. VPS scheduling and other host activity can
change results between runs. The commands above are a starting point; use your
intended worker count and configuration for deployment decisions.

The exact-count `--bench` mode also supports checksum verification. See
[offline benchmarks](CONFIGURATION.md#offline-benchmarks) for its options.

## Threads and memory

Automatic worker selection uses the CPUs and OS limits described in
[platform support](PLATFORMS.md). Explicit `-t N` workers are unbound unless
`--cpu-affinity` is supplied; JSON `cpu.rx` can specify per-worker CPU IDs.
Thread count, SMT and CPU placement can affect both throughput and memory use.

Full mode allocates a dataset; light mode retains the 256 MiB cache and computes
dataset items on demand. Each worker also needs a 2 MiB scratchpad. Backend
selection and memory-mode selection are independent choices.

NUMA placement can keep a separate dataset near each group of workers.
`--randomx-no-numa` uses one shared dataset. Dataset initialization temporarily
needs a cache; successful full-mode builds release it before mining. In `auto`
mode, a failed full-dataset allocation can reuse that cache for light mode.

Reserved-page options are preferences and can fall back to ordinary pages.
Check the actual allocation before attributing a difference to the backend.
Linux THP policy and scratchpad pooling provide additional memory controls;
see [configuration](CONFIGURATION.md) for their semantics and limits.

## Execution and OS controls

JIT/interpreter selection, hardware/software AES, secure code protection and
x86-64 prefetch settings are independent of full/light mode. Availability depends
on the CPU and platform. See [RandomX support](RANDOMX.md) for implementation
coverage and [configuration](CONFIGURATION.md) for the supported settings.

MSR writes are opt-in. Original values are restored on shutdown when
`randomx.rdmsr` is enabled. Keep MSR settings and other OS controls identical
when comparing builds.

Correctness tests and native differential validation are documented in the
[development guide](DEVELOPMENT.md).
