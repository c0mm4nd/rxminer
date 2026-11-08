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
by default and should be measured separately. Test builds are for local
regressions and must not be distributed.

## Threads and memory

On Linux, the default worker count uses allowed physical cores and respects
the CPU quota. Windows selects allowed physical cores in the current processor
group and binds automatic workers. macOS uses the physical-core count with OS
scheduling; see [platform support](PLATFORMS.md). Start with that setting, then measure an explicit `-t N` or `--smt` if
needed. Explicit `-t N` workers are unbound unless `--cpu-affinity` is supplied;
JSON `cpu.rx` can specify per-worker CPU IDs. More threads do not always
increase throughput.

NUMA mode keeps a dataset near each group of workers. `--randomx-no-numa` uses one
shared dataset. The miner can fall back to a shared dataset if replication
cannot be allocated. Dataset initialization temporarily needs a 256 MiB cache;
each worker also needs a 2 MiB scratchpad.
In `auto` mode, a failed full-dataset attempt transfers its initialized cache
to the light VM instead of freeing it and rebuilding another cache. This avoids
one 256 MiB allocation/release cycle and one cache initialization per fallback;
it does not reduce steady-state light-mode memory. Successful full-mode builds
still release their cache before mining. Each selected NUMA group keeps its own
cache, and cancellation does not start a fallback.
Explicit `light` mode also omits the cache's dataset-initialization JIT, which
light VMs never call. Worker VMs still select their own JIT normally, including
secure JIT. On Unix x86-64 this removes one 80 KiB initializer mapping per cache;
a Linux ordinary-page sample with `test key 000` had 32 KiB of resident code in
that mapping. Residency depends on the seed, architecture and page policy.

Transparent huge-page hints are enabled by default; `--no-thp` disables them.
Explicitly reserved 2 MiB pages are requested by default and fall back when
allocation is unavailable. Use `--no-huge-pages` to disable that request. The miner does not reserve system pages. MSR changes are opt-in and restored
on exit by default. Startup logs and the HTTP summary report actual explicit-page coverage.
Each dataset needs 1,040 reserved pages and each worker needs one. Optional 1 GiB dataset mappings require three reserved pages and fall back to
smaller pages when unavailable. JIT huge pages, scratchpad reuse and prefetch
mode selection are described in the [configuration reference](CONFIGURATION.md).

## Implemented optimizations

- JIT-assisted dataset initialization and runtime CPU-feature selection.
- Dataset initializers claim disjoint 4 MiB blocks as they finish work, reducing
  startup delays from slower or busier cores while retaining NUMA placement.
- Large-allocation alignment and huge-page hints; small VM/JIT objects use
  ordinary page alignment without unnecessary over-allocation.
- Pipelined hashing, reusable input buffers and cached task snapshots.
- Batched per-worker hash counters, separated by 128 bytes. Workers publish to
  their own slots; readers aggregate counts, including retired workers, without
  a shared atomic increment in the mining loop.
- Active-length program generation, avoiding unused v2 capacity for v1 work.
- Direct BLAKE2b processing for fixed-size register files, avoiding input
  buffer copies while preserving the general hashing API.
- Single-block BLAKE2b preparation for unkeyed inputs up to 128 bytes with
  32/64-byte digests, including seed expansion and typical mining inputs.
  The shared implementation covers x86-64 (Unix/MSVC), ARM64 and RV64 without
  adding CPU-extension requirements. Keyed and other digest sizes retain the
  streaming path; in-place and partially overlapping buffers are supported.
- Register-number normalization during program generation, removing a second
  per-instruction pass in the compiled VM path.
- JIT emission improvements, reciprocal constant hoisting and CPU-specific
  rounding helpers where supported.
- Full-memory Unix x86-64 VMs use a compact JIT buffer without the light-mode
  superscalar area. With default RandomX parameters, the code mapping shrinks
  from 80 KiB to 16 KiB per VM. The epilogue shares a page with the constant
  tables: on Linux with ordinary 4 KiB pages, constructor-time resident code
  pages measured 12 KiB before and 8 KiB after. The 64 KiB virtual-space saving
  is not a 64 KiB RSS saving. Explicit 2 MiB JIT pages still allocate 2 MiB;
  cache initialization and light VMs retain the general code buffer. The
  compact buffer supports v1/v2 and secure JIT with the same hash results.
- Unix x86-64 v1 JIT omits a rounding update when another unconditional update
  overwrites it before any rounding-dependent operation or branch. Bitwise
  floating-register swaps/scaling do not block this elimination; arithmetic,
  branches and the program boundary do. v2 retains its conditional updates.
- Loop-local interpreter dispatch, removing an external function call and
  repeated argument/PC handling for each decoded instruction. The retained
  single-step API uses the same instruction handlers. This improves the
  interpreter fallback; it does not change the JIT execution path.

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
Stop existing mining workers before performance comparisons, verify they have
actually stopped, and allow the machine to settle. CPU pinning alone does not
isolate shared caches, memory bandwidth or frequency from a running miner.
Requesting huge pages alone does not establish that they were allocated.

For bounded kernel comparisons on each native target:

```bash
RXMINER_SHORT_BENCH=1 cargo test --locked --release --test native_vectors -- --ignored --nocapture
```

The one-shot BLAKE2b probe compares against the streaming path with the same
compression kernel, compiler and optimization level. It alternates execution
order and reports medians for 64/76/128-byte inputs and the existing 256-byte
register-file path. The three BLAKE2b/AES probes and the interpreter-dispatch
probe each have a 500 ms sampling budget, checked between bounded batches;
correctness checks run separately.
CI records these samples on its native platforms, including the manually
dispatched RISC-V workflow. Kernel timing improvements are not overall
hashrate improvements; ARM64, Windows and RV64 results require those runners.

## Whole-hash flow

The current profile hashes the input into a 64-byte seed, fills a 2 MiB
scratchpad, then executes eight generated programs. Each program runs 2,048
iterations (256 instructions in v1, 384 in v2, plus taken branches). Between
programs, BLAKE2b hashes the register file into the next seed. After the final
program, an AES scan of the scratchpad replaces register `a`, and BLAKE2b
produces the 32-byte result.

These dependencies constrain optimization: program N+1 needs the result of
program N, and the final scratchpad scan needs all VM writes. Skipping programs,
iterations or scratchpad bytes changes the hash. The miner already combines
the current hash's scratchpad scan with filling the next hash's scratchpad in
one traversal. Further work should target overhead inside these required stages.

For example, moving interpreter dispatch inside its execution loop reduces
the base v1 dispatch-call count from 4,194,304 per hash to 16,384 whole-program
calls; taken branches no longer add external dispatch calls. It preserves the
program counter, rounding changes and every instruction's memory effects.
The regression compares complete register and scratchpad state, including
branches to the start/end and v1/v2 rounding behavior, against single stepping.

The optional Linux x86-64 developer probe measures the actual stages with a
real cache/dataset and checks every pipelined result against the single-hash API:

```bash
cmake -S . -B target/flow-native -DCMAKE_BUILD_TYPE=Release
cmake --build target/flow-native --target rxbench-flow -j 2
target/flow-native/rxbench-flow jit full v1 64
target/flow-native/rxbench-flow jit full v2 16 secure
target/flow-native/rxbench-flow interpreter light v1 16
```

Arguments select `jit|interpreter`, `light|full`, `v1|v2`, 1–64 hashes,
and optionally `default|secure` (secure requires JIT).
Full mode needs roughly 2.3 GiB and initializes the dataset with four workers.
Hashing is then pinned to the first CPU allowed by the process affinity mask.
Initialization and warmup are excluded from timing. Reported stages separate
input hashing, initial fill, VM chains (including generation/compilation),
chain hashing, fused finish/refill, and the final drain. Small stage timers
add overhead, and the single-hash comparison runs first; this probe diagnoses
where time goes, rather than replacing alternating whole-miner benchmarks.

Full-memory JIT probes also separate program generation, VM initialization,
code compilation, execution, and writable/executable permission transitions.
A benchmark-only VM subclass times those phases; reference hashes call the
original VM implementation on the same object with instrumentation disabled.
Every resulting hash must match. The subclass adds no object storage and the
production VM contains no timers. Light/interpreter modes retain aggregate
VM-chain timing. Extra phase timers add overhead, particularly for short stages.

For paired full-memory checks, `rxbench-compare` loads two builds with the same
native VM-base and dataset ABI into one process, shares a real initialized
dataset, pins hashing to one allowed CPU and compares every 32-byte hash.
It crosses execution order with scratchpad backing: outside timing, Linux
`mremap` exchanges the two ordinary 2 MiB mappings without changing VM pointers.
Each next batch refills the scratchpad before execution. Build this optional
Linux target with:

```bash
cmake --build target/flow-native --target randomx rxbench-compare -j 2
c++ -shared -Wl,-Bsymbolic -Wl,--whole-archive target/flow-native/librandomx.a \
  -Wl,--no-whole-archive -pthread -lm -o target/before.so
# After a change, rebuild with the same flags and link the archive as target/after.so.
target/flow-native/rxbench-compare target/before.so target/after.so 12 256 v1
target/flow-native/rxbench-compare target/after.so target/before.so 12 256 v1
target/flow-native/rxbench-compare target/before.so target/before.so 12 256 v1
```

The final arguments select sample pairs (1–32, default 8), hashes per batch
(1–1,024), and v1/v2. Four warmup pairs are excluded. Use a multiple of four
measured pairs to balance both execution orders and both scratchpad mappings.
Per-build times are medians; paired time ratios use a geometric mean so that
reciprocal mapping effects cancel instead of biasing an arithmetic midpoint.
A ratio above one favors the second library. Wall time and thread CPU time
are reported separately; CPU time excludes
scheduler preemption but still reflects frequency, memory and cache effects.
VM and JIT virtual addresses remain distinct, so reverse-order and same-build
controls are still necessary before attributing a small difference to an
optimization. Timings never gate correctness or establish a universal speedup.
Mapping exchanges also disturb translation/cache state; single-hash batches
are useful for drain-path correctness, not performance conclusions. Use longer
batches and check the same-build control at the chosen batch size.

An audit found a 96-thread miner running alongside the initial local timing
experiments. Those timings, including the floating-point/integer scheduling
trials, must not be used as reliable performance evidence. The exercised
state/hash correctness checks remain valid; the scheduling variants were not
retained. The retained swap regression covers all register pairs, taken branches,
temporary-register users, raw/normalized operands, and v1/v2 with
software/hardware AES.

After stopping that miner on the AMD EPYC 7K62, a pinned 64-hash v1 full-JIT
phase sample attributed about 96.5% of pipeline time to generated-code execution
and 1.2% to compilation. This supports investigating the execution loop before
further seed/generator micro-optimizations; it is not a throughput comparison.
An early dataset-prefetch trial issued an extra hint after the final relevant
register write and branch. Forward/reverse comparisons (12 pairs of 256 hashes)
showed no stable benefit above same-build variation, so it was reverted.
Replacing integer `XCHG` with three `MOV`s was also retested without the miner
(12 pairs of 512 hashes in each order); the two orders did not show a consistent
benefit, so the original instruction remains.

## Concurrency and validation

Each worker owns its VM and shares an initialized dataset through `Arc`.
Seed changes join workers before replacing datasets. Hash counts are flushed
on exit, and unchanged jobs preserve nonce progress when work resumes.
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
