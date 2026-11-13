# RandomX

## Source and license

- Upstream: [tevador/RandomX](https://github.com/tevador/RandomX).
- Base commit: `daecc162b91fd0b0af2da8520baf16a936c9f374`.
- License: [BSD 3-Clause](LICENSE).
- Selected optimizations are adapted from BSD-licensed files in XMRig commit
  `b2ca72480c58d197e18c885d9fc1a0c8d517e60a`, including
  `crates/randomx-cpp/native/jit_compiler_x86.cpp` and `aes_hash.cpp`.
  Original author notices are retained in the adapted files and packaged
  license.

## Maintenance

RandomX is maintained in-tree under `crates/randomx-cpp/native/`. Source changes and
upstream updates are reviewed and committed with the miner; there is no
submodule or build-time source download. Keep upstream attribution and local
regression coverage when applying updates.

These sources are bundled and built by the `randomx-cpp` crate when the miner
selects `--no-default-features --features randomx-cpp`. The default miner uses
the independent `randomx-rust` crate. The optional Linux x86-64 differential
reference is built through `cmake/randomx.cmake`. The notes below describe the
native backend.

Windows uses the upstream x64 JIT and ABI-specific assembly. Unix x86-64 keeps
the local optimized JIT. ARM64 uses the upstream JIT with a dedicated executable
allocator; Apple Silicon requires per-thread write/execute protection. The
portable interpreter remains the fallback for architectures without a backend.
RV64 uses the upstream scalar JIT and dataset initializer with the RV64GC
instruction baseline. Vector dispatch and signal-based instruction probing are
omitted; no vector or crypto extension is required. Generated code retains
instruction-cache synchronization, and each VM owns its code and literal buffers.

## Local changes

- Linux memory allocation aligns large objects for transparent huge-page hints.
  Small VM/JIT objects use ordinary page alignment. Explicit 2 MiB allocation
  uses checked writable prefaulting before exposing memory to workers.
- VM construction reads CPU capabilities without a shared mutable AES probe.
- JIT code uses a fixed instruction-offset buffer, typed dispatch and local
  emission cursors. Runtime-gated BMI2 helpers, short branches and Zen 2 v1
  rounding helpers reduce preparation or execution work.
- Full-memory Unix x86-64 VMs omit the unused superscalar code region and place
  their epilogue beside the constant tables. The default-profile mapping is
  16 KiB instead of 80 KiB; ordinary 4 KiB pages also avoid a separate resident
  epilogue page. Light VMs and cache initialization keep the general layout,
  and explicit JIT huge-page allocation remains rounded to 2 MiB.
- The Unix x86-64 JIT removes v1 rounding updates overwritten before any use.
  Arithmetic, branches and the program boundary stop the forward scan; v2
  conditional rounding updates are retained.
- Reciprocal constants are hoisted into VM-private storage. Leaf System V
  VMs can use the stack red zone; calling paths use a private code table.
  Secure JIT writes code and constants before enabling execution.
- Program generation fills only the active v1/v2 length. A separate generator
  normalizes stored register fields for the Unix x86 compiled VM while preserving
  the entropy header, AES feedback and raw-generator interface.
- Unkeyed BLAKE2b with a 256-byte input and 32/64-byte output processes the
  register file directly. Inputs up to 128 bytes also bypass streaming-state
  preparation for 32/64-byte output, including 64-byte seed expansion. These
  shortcuts apply to every architecture; other sizes and keyed inputs retain
  the streaming path. Overlapping and unaligned buffers remain supported.
- The AES hash/fill loop processes two blocks per iteration. Optional VAES
  kernels are disabled by default and selected at runtime when enabled.
- Interpreter opcode dispatch is inlined into the bytecode loop, avoiding
  a function boundary per executed instruction. Single stepping shares the
  same handlers and remains available for differential validation.

These changes do not alter the hash algorithm or introduce shared mutable
state between VMs. Performance should be measured on the target system.

## Validation

See the [development guide](../../../docs/DEVELOPMENT.md#optional-native-differential-tests)
for the explicit CMake build and oracle environment variables. The ignored
Cargo differential tests only execute/load that prebuilt reference. Ordinary
Cargo builds with the default Rust backend require no RandomX C/C++ compilation.
