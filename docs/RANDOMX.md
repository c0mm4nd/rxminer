# RandomX support

RxMiner is a Rust CPU miner for `rx/0` pools. Its owned FFI layer uses the
integrated RandomX C/C++ library for hashing and JIT execution. See [platform support](PLATFORMS.md) for the native backends and validation status.

## Supported behavior

- Full-memory RandomX, JIT compilation and runtime hardware AES selection.
- CPU affinity, per-node NUMA datasets and a shared-dataset fallback.
- Transparent huge-page hints and optional explicit 2 MiB pages.
- TLS or plaintext Stratum, with HTTP CONNECT and SOCKS5 proxy support.
- A built-in developer fee below 1%; benchmark mode has no pool connection
  or fee.

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

## Validation

`cargo test --locked` checks Rust behavior and known hash vectors.
`cargo test --locked --release --test tooling native_regressions -- --ignored`
runs the native and differential regressions against the library built by Cargo.
Full-dataset tests compare worker output with a light VM. Local protocol tests
use Rust pool and proxy fixtures in `tests/support/`.

See the [development guide](DEVELOPMENT.md) for commands and the
[performance guide](OPTIMIZATION.md) for configuration and measurement.
