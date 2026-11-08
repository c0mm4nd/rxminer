# RandomX support

RxMiner is a Rust CPU miner for `rx/0` pools. Its owned FFI layer uses the
integrated RandomX C/C++ library for hashing and JIT execution. See [platform support](PLATFORMS.md) for the native backends and validation status.

## Supported behavior

- Full-memory RandomX, JIT compilation and runtime hardware AES selection.
- CPU affinity, per-node NUMA datasets and a shared-dataset fallback.
- Transparent huge-page hints and optional explicit 2 MiB pages.
- TLS or plaintext Stratum, with HTTP CONNECT and SOCKS5 proxy support.
- Offline benchmarks without a pool connection.

The miner accepts `rx/0` jobs. Native regression coverage also exercises
RandomX v2, but this does not enable v2 pool jobs or other mining algorithms.
The Monero-style profile uses nonce offset 39 and CryptoNote share-target
encoding. Other coins may require different header layouts, seed schedules
or pool protocols.

## Source and license

The hash library is based on [tevador/RandomX](https://github.com/tevador/RandomX)
commit `daecc162b91fd0b0af2da8520baf16a936c9f374`. Local changes and third-party
attribution are described in the [fork notes](../src/crypto/randomx/README.md).
The library's BSD 3-Clause license is included in source and binary packages.

## Rust migration

`src/randomx_core/` is the portable Rust foundation for an eventual replacement
of the native backend. It has no external crate dependencies and forbids unsafe
code. Its source retains the RandomX attribution and BSD license. The current
miner still executes the native VM; this module does not yet calculate complete
RandomX hashes and makes no mining performance claim.

The first stage implements:

- Unkeyed BLAKE2b with 1–64-byte digests, streaming updates and Argon2 H'.
  Hash state and expansion use fixed-size buffers without heap allocation.
- The BLAKE2 byte generator, including 60-byte seed truncation, little-endian
  nonce encoding and discarded bytes when a word read crosses a block boundary.
- Superscalar integer execution, signed/unsigned high multiplication and
  reciprocal multiplication. Raw divisors and prepared reciprocal tables are
  both accepted; decoding validates inputs and prepares constant operands before
  execution. This is not yet the superscalar program generator/scheduler.

`cargo test --locked --test randomx_rust` compares hashing and reciprocals with
the native library built from this checkout. The portable native vector suite
also builds a C++ oracle for 160 mixed-read generator streams and 3,840 generated
or crafted superscalar programs, checking all registers in both reciprocal modes.
Only the test executable uses that oracle; the Rust core does not call C/C++.

The foundation can be tested independently of Cargo, TLS and native RandomX:

```bash
rustc --edition=2021 --test src/randomx_core/mod.rs -o /tmp/randomx-core-tests
/tmp/randomx-core-tests
```

Next stages are the superscalar scheduler, Argon2d cache and dataset construction,
AES program/scratchpad generation, the full interpreter and hash pipeline, then
architecture-specific JITs. Each stage must match intermediate states as well as
known final hashes. Floating-point rounding, v1/v2 differences, full/light modes
and the existing allocation/JIT optimizations need explicit validation before
the production backend can be replaced.

## Validation

`cargo test --locked` checks Rust behavior and known hash vectors.
`cargo test --locked --release --test tooling native_regressions -- --ignored`
runs the native and differential regressions against the library built by Cargo.
Full-dataset tests compare worker output with a light VM. Local protocol tests
use Rust pool and proxy fixtures in `tests/support/`.

See the [development guide](DEVELOPMENT.md) for commands and the
[performance guide](OPTIMIZATION.md) for configuration and measurement.
