# Performance and validation

RxMiner uses portable builds by default. Performance depends on the CPU,
thread count, memory placement and available page sizes; measure on the target
machine rather than relying on a fixed speedup claim.

## Build

```bash
cargo build --release --locked
```

For a binary used only on the build machine:

```bash
RXMINER_ARCH=native RUSTFLAGS='-C target-cpu=native' cargo build --release --locked
```

Use the portable build for distribution. Cargo builds the native RandomX
library directly; CMake, Make and Ninja are not required. The optional `vaes`
feature enables runtime-dispatched VAES kernels on x86-64 with GCC or Clang;
other targets use the default AES kernels. It is off
by default and should be measured separately. Never distribute a build with
`test-dev-fee` enabled.

## Threads and memory

On Linux, the default worker count uses allowed physical cores and respects
the CPU quota. Other targets currently use OS scheduling and available logical
processors; see [platform support](PLATFORMS.md). Start with that setting, then measure an explicit `-t N` or `--smt` if
needed. Explicit `-t N` workers are unbound unless `--cpu-affinity` is supplied;
JSON `cpu.rx` can specify per-worker CPU IDs. More threads do not always
increase throughput.

NUMA mode keeps a dataset near each group of workers. `--randomx-no-numa` uses one
shared dataset. The miner can fall back to a shared dataset if replication
cannot be allocated. Dataset initialization temporarily needs a 256 MiB cache;
each worker also needs a 2 MiB scratchpad.

Transparent huge-page hints are enabled by default; `--no-thp` disables them.
Explicitly reserved 2 MiB pages are requested by default and fall back when
allocation is unavailable. Use `--no-huge-pages` to disable that request. The miner does not reserve system pages. MSR changes are opt-in and restored
on exit by default. Startup logs and the HTTP summary report actual explicit-page coverage.
Each dataset needs 1,040 reserved pages and each worker needs one. Optional 1 GiB dataset mappings require three reserved pages and fall back to
smaller pages when unavailable. JIT huge pages, scratchpad reuse and prefetch
mode selection are described in the [configuration reference](CONFIGURATION.md).

## Implemented optimizations

- JIT-assisted dataset initialization and runtime CPU-feature selection.
- Large-allocation alignment and huge-page hints; small VM/JIT objects use
  ordinary page alignment without unnecessary over-allocation.
- Pipelined hashing, reusable input buffers, cached task snapshots and batched
  hash counters.
- Active-length program generation, avoiding unused v2 capacity for v1 work.
- Direct BLAKE2b processing for fixed-size register files, avoiding input
  buffer copies while preserving the general hashing API.
- Register-number normalization during program generation, removing a second
  per-instruction pass in the compiled VM path.
- JIT emission improvements, reciprocal constant hoisting and CPU-specific
  rounding helpers where supported.

These changes preserve hash results. Reducing preparation work does not imply
the same percentage increase in overall hashrate. Native implementation and
source attribution are documented in the
[RandomX fork notes](../src/crypto/randomx/README.md).

## Local measurement

The built-in benchmark needs no pool connection:

```bash
./target/release/rxminer --benchmark-seconds 60
./target/release/rxminer --benchmark-seconds 60 --no-thp
```

Change one setting at a time. Keep the seed, CPU affinity, thread count and
page policy consistent when comparing builds. Repeat measurements after
initialization and warmup, and account for other activity on the machine.
Requesting huge pages alone does not establish that they were allocated.

## Concurrency and validation

Each worker owns its VM and shares an initialized dataset through `Arc`.
Seed changes join workers before replacing datasets. Hash counts are flushed
on exit, and unchanged jobs preserve nonce progress across fee windows.
Worker failures notify the coordinator and roll back partially started work.
TLS and proxy handshakes poll cancellation with bounded deadlines. Stopping
also shuts down registered sockets.

Tests cover known hash vectors, raw and normalized JIT paths, v1/v2 switching,
full-memory/light-VM agreement, secure JIT, worker failures and task changes.
BLAKE2b tests also cover keyed fallback, unaligned buffers and overlapping
input/output. These checks cover exercised paths, not every possible thread
interleaving. Cancellation may wait for a native hash or initialization chunk;
blocking DNS and an unregistered TCP connection attempt are not forcibly
interrupted.

Build, correctness, protocol and installation checks are documented in the
[development guide](DEVELOPMENT.md). Full-dataset tests need at least 4 GiB of
free memory. Run expensive tests sequentially.
