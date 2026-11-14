# RxMiner v0.2.0-rc.4

Fourth release candidate for v0.2.0, the AI-optimized RandomX miner. This is a
pre-release for validation before the stable v0.2.0 release.

## Changes since rc.3

- Keep user mining active when the developer pool disconnects, rejects login,
  has no job, or reports a different RandomX seed. Developer windows reuse the
  user's dataset; missed developer time is not made up after recovery.
- Preserve the existing pause and recovery behavior when user work is
  unavailable. Developer endpoint failures no longer require both pools to
  recover before user work can resume.
- Use the intended MXCSR save/restore path in the C++ bridge on SSE2 platforms,
  avoiding full libc floating-point environment calls for each pipelined hash.
  Exception handling and exact caller floating-point state restoration remain
  covered by the backend contract tests.
- Add developer-only outage and recovery regression coverage, including
  continued user shares, nonce continuity, and dataset reuse. Both backends
  retain the existing TLS, proxy, runtime control and supervisor checks.

## Changes since v0.1.1

- Maintain separate `randomx-rust` and `randomx-cpp` crates, selected at build
  time. Rust is the default; both backends support full/light memory modes,
  interpreter/JIT execution and the same miner CLI and configuration.
- Extract CPU topology, affinity, NUMA and memory handling into `rx-platform`.
- Use rustls for TLS, removing the OpenSSL library dependency. TLS still uses
  ring's native cryptographic kernels.
- Keep retrying user and developer pool connections through network outages.
  Mining and the fee clock pause when user jobs are unavailable.
- Run under a built-in Rust foreground supervisor by default. Failed mining
  sessions are rebuilt; crashed or killed mining children are restarted with
  bounded delays. Explicit signals and authenticated API stop remain supported.
- Preserve configured proxy and TLS settings during recovery. A failed proxy
  does not cause a fallback to direct connections.
- Produce separately named Rust/C++ archives, with build identity metadata,
  individual SHA-256 sidecars and a combined `SHA256SUMS` file.

## Packages

Choose the platform and backend in the filename:

`rxminer-v0.2.0-rc.4-<platform>-<rust|cpp>.<tar.gz|zip>`

Both variants are built for Linux x86-64, ARM64 and RV64, macOS Intel and Apple
Silicon, and Windows x64. Each archive is extracted and tested before upload;
the complete matrix and checksums are verified before a release draft is made.

Rust Linux packages do not require the C++ runtime. C++ packages may depend on
the system C++ runtime. Neither backend links OpenSSL. Windows archives include
Visual C++ runtime DLLs; macOS binaries are ad-hoc signed. Linux archives are
GNU/glibc builds from Ubuntu 24.04, not static musl or older-glibc builds.

## Operational changes

The default foreground invocation now has a supervisor process and a mining
child. Ctrl+C, SIGTERM and API stop end supervision; benchmarks and configuration
validation retain their normal finite exit behavior. `--background` remains an
explicit opt-in.

Source builds select one backend, for example:

```bash
cargo build --release --locked --no-default-features --features randomx-rust
cargo build --release --locked --no-default-features --features randomx-cpp
```

Do not enable the local-fixture `test-dev-fee` feature in production builds.
