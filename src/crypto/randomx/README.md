# RandomX

## Source and license

- Upstream: [tevador/RandomX](https://github.com/tevador/RandomX).
- Base commit: `daecc162b91fd0b0af2da8520baf16a936c9f374`.
- License: [BSD 3-Clause](LICENSE).
- Selected optimizations are adapted from BSD-licensed files in XMRig commit
  `b2ca72480c58d197e18c885d9fc1a0c8d517e60a`, including
  `src/crypto/randomx/jit_compiler_x86.cpp` and `aes_hash.cpp`.
  Original author notices are retained in the adapted files and packaged
  license.

## Maintenance

RandomX is maintained in-tree under `src/crypto/randomx/`. Source changes and
upstream updates are reviewed and committed with the miner; there is no
submodule or build-time source download. Keep upstream attribution and local
regression coverage when applying updates.

Cargo compiles these sources directly through `build.rs`. The source package
includes the implementation and license. Regression tests live in
`tests/randomx/`; upstream examples and analysis tools are omitted. Windows x64 and ARM64
backends are restored from the same pinned upstream commit. The optional native developer build is configured by
`cmake/randomx.cmake`. Run validation commands from the repository root.

Windows uses the upstream x64 JIT and ABI-specific assembly. Unix x86-64 keeps
the local optimized JIT. ARM64 uses the upstream JIT with a dedicated executable
allocator; Apple Silicon requires per-thread write/execute protection. The
portable interpreter remains the fallback for architectures without a backend.

## Local changes

- Memory allocation aligns large objects for transparent huge-page hints.
  Small VM/JIT objects use ordinary page alignment. Explicit 2 MiB allocation
  uses checked writable prefaulting before exposing memory to workers.
- VM construction reads CPU capabilities without a shared mutable AES probe.
- JIT code uses a fixed instruction-offset buffer, typed dispatch and local
  emission cursors. Runtime-gated BMI2 helpers, short branches and Zen 2 v1
  rounding helpers reduce preparation or execution work.
- Reciprocal constants are hoisted into VM-private storage. Leaf System V
  VMs can use the stack red zone; calling paths use a private code table.
  Secure JIT writes code and constants before enabling execution.
- Program generation fills only the active v1/v2 length. A separate generator
  normalizes stored register fields for the x86 compiled VM while preserving
  the entropy header, AES feedback and raw-generator interface.
- Unkeyed BLAKE2b with a 256-byte input and 32/64-byte output processes the
  register file directly. Other sizes and keyed inputs retain the streaming
  path. Overlapping and unaligned buffers remain supported.
- The AES hash/fill loop processes two blocks per iteration. Optional VAES
  kernels are disabled by default and selected at runtime when enabled.

These changes do not alter the hash algorithm or introduce shared mutable
state between VMs. Performance should be measured on the target system.

## Validation

```bash
cargo test --locked --release --test tooling native_regressions -- --ignored
```

The driver links the native archive built by Cargo and runs the upstream
suite plus JIT state, program generation and BLAKE2b differential checks.
Coverage includes raw/normalized programs, v1/v2 switching, software/hardware
AES, reciprocal boundaries, keyed fallback and overlapping buffers.
Hardware-dependent cases may be skipped when unavailable.

Rust tests additionally cover known vectors, full-memory/light-VM agreement,
secure JIT and worker lifecycle behavior. See the
[performance guide](../../../docs/OPTIMIZATION.md) for commands. CMake is not
required to build, install or run the Cargo-based checks.
