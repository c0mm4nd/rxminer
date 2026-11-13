# RandomX support

RxMiner is an AI-optimized RandomX miner for `rx/0` pools. Its default hashing backend is the
Rust JITs in `crates/randomx-rust/src/randomx_jit/` on x86-64, ARM64 and Linux RV64, with a selectable
portable interpreter in `crates/randomx-rust/src/randomx_core/`. Both support full/light memory modes,
parallel dataset initialization, CPU affinity and NUMA placement.

Cargo selects `randomx-rust` by default. Use `--no-default-features --features
randomx-cpp` to build the independently maintained C++ crate instead. The Rust
crate has no native build dependencies; the C++ crate owns its `cc` build script
and bundled sources. TLS still uses ring in both configurations.

The miner accepts `rx/0` jobs. Native regression coverage also exercises
RandomX v2, but this does not enable v2 pool jobs or other mining algorithms.
The Monero-style profile uses nonce offset 39 and CryptoNote share-target
encoding. Other coins may require different header layouts, seed schedules
or pool protocols.

## Source and license

The hash library is based on [tevador/RandomX](https://github.com/tevador/RandomX)
commit `daecc162b91fd0b0af2da8520baf16a936c9f374`. Local changes and third-party
attribution are described in the [fork notes](../crates/randomx-cpp/native/README.md).
The Rust core's retained BSD 3-Clause license is included in source and binary packages.

## Rust migration

`crates/randomx-rust/src/randomx_core/` implements complete portable RandomX v1/v2 hashing. It has no
external crate dependencies and forbids unsafe code. Its source retains the
RandomX attribution and BSD license. The miner selects the Rust or C++ crate through mutually exclusive features.
Within the Rust crate, JIT availability is selected by platform; the interpreter
remains selectable at runtime.

On x86-64, interpreter mode uses a native SSE2 executor for arithmetic and
explicit MXCSR rounding changes, restoring the caller's entire MXCSR on return.
It executes decoded data without generating executable code. The safe core's
integer binary64 implementation remains an independent reference and the
fallback for other platforms. Software AES uses immutable lookup tables and a
single scratchpad traversal for combined hash/refill; the x86-64 backend uses
SSE2 registers without requiring AES-NI.

The x86-64, ARM64 and RV64 emitters implement every VM instruction, superscalar
execution and parallel dataset initialization. Templates use Rust `global_asm!`
and LLVM's integrated assembler; normal builds neither invoke an external
assembler nor link a C++ RandomX library. Full datasets use OS-zeroed mappings
without a serial clearing pass; errors and cancellation never publish partial data.

The retained C++ algorithm features are available through Rust:

| Capability | Rust implementation |
| --- | --- |
| Full/light; RandomX v1/v2 | Interpreter and all three JITs |
| Software AES | Every platform and execution mode |
| Hardware AES | AES-NI on x86-64; ARMv8 AES on ARM64, including interpreter mode |
| Argon2 kernels | Scalar everywhere; SSSE3/AVX2 and runtime selection on x86-64 |
| Reserved memory | 2 MiB cache/dataset/scratchpads; Linux 1 GiB dataset preference |
| Memory policy | Actual page reporting, Linux THP advice, bounded NUMA/page-aware scratchpad pool |
| JIT controls | Secure write protection, supported huge-page allocation, x86-64 prefetch 0..3 |
| Lifecycle | Cache reinitialization, dataset range initialization, VM source replacement |
| Hash API | Single hash, first/next/last pipeline, BLAKE2b-256 commitment |

`randomx.jit` and `randomx.secure` select execution and code protection independently
of full/light mode. Page options are preferences and report their actual fallback.
A VM source can be replaced between hash pipelines while preserving full/light mode.
Rust ownership prevents cache mutation while a VM still shares it.

The x86-64 emitter retains BMI2 selection, reciprocal constants, short branches
and Zen 2 rounding optimizations. The v2 software AES JIT keeps its four lanes
in SSE2 registers; Zen 2 light-mode Superscalar cache reads use T0 independently
of the scratchpad-prefetch setting. Bulk dataset initialization retains NTA. Relocatable template entries are explicitly aligned
to preserve SIMD constant alignment. The emitters preserve the caller's FP control
and exception state. Ordinary code backing is 16/80 KiB for x86-64 full/light,
32/128 KiB for ARM64 and 64/256 KiB for RV64. Linux x86-64/ARM64 normally use
separate RW/RX aliases; secure mode uses RW/RX transitions on one mapping. RV64
always uses a single mapping with process-wide instruction synchronization.
Windows uses VirtualProtect and FlushInstructionCache; Apple Silicon uses MAP_JIT
with per-thread write protection and cache invalidation. Reserved code pages
have the OS page granularity, which may exceed these ordinary backing sizes. Light x86-64
VMs discard the helper staging copy after publication, retaining only a 16 KiB
program staging buffer; this also applies after replacing their cache.

Native Linux differential tests cover 72 combinations of full/light,
interpreter/JIT, software/hardware AES, secure mode, x86-64 prefetch and v1/v2,
including pipelines, source replacement and commitment. ARM64/RV64 tests execute
all instruction classes, random mixed programs, all eight chained programs and
full/light pipelines under local QEMU. Cross-compilation checks cover Windows
and both macOS architectures; native runtime and reserved-huge-page success
paths still require machines configured for those facilities. See
[platforms](PLATFORMS.md) and [development](DEVELOPMENT.md) for scope and commands.
The Rust core currently implements:

- Unkeyed BLAKE2b with 1–64-byte digests, streaming updates and Argon2 H'.
  Hash state and expansion use fixed-size buffers without heap allocation.
- The BLAKE2 byte generator, including 60-byte seed truncation, little-endian
  nonce encoding and discarded bytes when a word read crosses a block boundary.
- Superscalar integer execution, signed/unsigned high multiplication and
  reciprocal multiplication. Raw divisors and prepared reciprocal tables are
  both accepted; decoding validates inputs and prepares constant operands before
  execution.
- Superscalar program generation, including the fixed Ivy Bridge decoder and
  execution-port model, register dependencies, operand stalls, discarded
  instructions and address-register selection. Register candidates use fixed
  stack buffers; the instruction tables require no heap allocations. The
  generated instruction buffer is allocated once per program.
- Argon2d's fixed RandomX configuration: 256 MiB, one lane, three passes and
  zero-length output tag. Cache reinitialization reuses its allocation.
- Dataset item calculation and full dataset construction. Parallel workers claim
  disjoint slices from a queue; CPU/NUMA setup and cancellation hooks are exposed
  to the miner. Light-mode fallback retains the initialized cache.
- Software AES rounds, scratchpad fill/hash and program generation.
- The full interpreter, v1/v2 differences, eight-program hash chains and
  first/next/last hashing. Floating-point operations use integer arithmetic with
  four explicit rounding modes, without changing the host floating-point state.

Ignored differential tests load an explicitly built native reference library or
run external reference executables. See [development](DEVELOPMENT.md) for the
CMake commands and required environment variables. Ordinary Cargo builds and
tests never compile that reference. Comparisons cover hashing and reciprocals,
160 mixed-read generator streams and 3,840 generated or crafted superscalar
programs, checking all registers in both reciprocal modes.
An additional 256 chains of eight programs (2,048 programs) compare exact
instruction bytes, address registers, scheduling metrics, execution results and
the remaining generator stream. The standalone Rust tests also check the ten
consecutive program hashes shipped in the upstream RandomX test suite.
The VM oracle additionally checks 81,920 floating-point results in all four
rounding modes, 8,192 AES rounds, full AES buffers, four complete cache digests,
4,096 dataset items and 24 complete v1/v2 hashes. The complete hashes also check
Rust JIT hardware/software AES paths using AVX2-initialized caches. Linux unit
tests compare entire AVX2/scalar caches, full/light JIT pipelines, exact MXCSR
restoration, the non-BMI2 path and code-map permissions including the `mprotect`
fallback. Compile-only tests saturate every opcode across emitter variants to
check code bounds and preservation of light-mode helpers. Standalone tests also
construct the complete dataset and compare full/light hashing and sequential VM reuse.

The foundation can be tested independently of Cargo, TLS and native RandomX:

```bash
rustc --edition=2021 -O --test crates/randomx-rust/src/randomx_core/mod.rs -o /tmp/randomx-core-tests
/tmp/randomx-core-tests
/tmp/randomx-core-tests --ignored --test-threads=1
```

The ignored tests need at least 3 GiB of free memory and several minutes for the
complete dataset. Remaining validation includes native Windows/macOS and ARM64/RV64
runs, reserved huge-page success paths and broader hardware/performance comparisons.
Keep the C++ reference available for differential tests.

## Validation

`cargo test --locked --release` checks Rust behavior and known vectors.
Ignored core tests check complete hashes, full/light agreement and sequential VM
reuse. Local protocol tests use Rust pool and proxy fixtures in `tests/support/`.
The C++ sources are packaged with `randomx-cpp`. A separate CMake shared-library
build is also available for independent differential validation.

See the [development guide](DEVELOPMENT.md) for commands and the
[tuning guide](OPTIMIZATION.md) for configuration and measurement.
