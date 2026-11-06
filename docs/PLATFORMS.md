# Platform support

Platform support is being expanded. The new targets below are experimental
until their native CI builds, hash checks and installation checks pass.

| Target | Hash backend | Status |
| --- | --- | --- |
| Linux x86-64 | Optimized x86-64 JIT | Existing supported target |
| Linux ARM64 | Upstream ARM64 JIT | Initial port; CI validation pending |
| macOS Intel | Optimized x86-64 JIT | Initial port; CI validation pending |
| macOS Apple Silicon | ARM64 JIT with write/execute protection | Initial port; CI validation pending |
| Windows x64 (MSVC) | Upstream Windows x64 JIT | Initial port; CI validation pending |

All targets build RandomX directly through Cargo. No submodule checkout,
build-time source download, CMake, Make, Ninja or Python is used by the miner
build. A native C/C++ toolchain and OpenSSL development libraries are required.

## Dependencies

- Linux: GCC or Clang, `pkg-config`, and OpenSSL headers (`libssl-dev` on Ubuntu).
- macOS: Xcode Command Line Tools, `brew install openssl@3 pkg-config`, and
  `OPENSSL_DIR="$(brew --prefix openssl@3)"` when installing with Cargo.
- Windows: Rust's MSVC toolchain, Visual Studio C++ Build Tools with the x64
  assembler, and OpenSSL headers and x64 libraries matching the CRT. With a
  full precompiled OpenSSL installation, set `OPENSSL_DIR` to its root,
  `OPENSSL_LIB_DIR` to its `lib/VC/x64/MD` directory, and `OPENSSL_STATIC=0`.
  Add its `bin` directory to `PATH` so the installed miner can load the DLLs.
  CI validates this installation path. A matching vcpkg installation is also
  supported by the OpenSSL dependency's discovery mechanism.

The Git installation command in the [README](../README.md#installation) is the
same on each OS after installing its dependencies. Cross-compilation requires
a suitable target compiler, assembler, linker and OpenSSL installation;
`RXMINER_ARCH=native` is only accepted for native builds.

## Current differences

Linux retains physical-core discovery, cgroup limits, NUMA placement, MSR
control and battery monitoring. On other platforms, automatic worker selection
uses the available logical processors and OS scheduling with a single dataset.

Windows can bind workers within the process's current processor group and
supports thread priorities, Ctrl+C/Ctrl+Break shutdown, logging and atomic
configuration replacement. Multi-group CPU discovery, NUMA placement, MSR
control, background mode and syslog are not implemented.

macOS uses unbound workers because it does not expose Linux-style hard CPU
affinity. Explicit CPU IDs and masks are rejected; use `-t N` or `cpu.rx`
entries of `-1`. Available-memory headroom is currently unknown on macOS;
use light mode when a full dataset cannot be allocated.

MSR tuning requires Linux x86-64. Battery monitoring and 1 GiB pages currently
require Linux. Nondefault JIT prefetch modes require the Unix x86-64 backend.
Unsupported explicit options fail validation. Huge-page requests retain their
ordinary-page fallback when allocation is unavailable.

GPU mining and other algorithms are not part of this port. RISC-V and Windows
ARM64 have not been validated. Release archives remain Linux x86-64 only;
other platforms use Cargo installation.

## Validation

Every new CI target runs Rust checks, known hash vectors, JIT/interpreter
agreement, full-memory and light-mode mining, and local Cargo installation.
Full-memory shares are checked against the light interpreter across concurrent
job updates, developer-fee switches and seed changes.
The existing Linux x86-64 jobs additionally run the native differential suite,
local pool/proxy/runtime regressions and fresh Git-install checks.

Hosted runners validate functional behavior. They do not establish comparable
mining performance or guarantee privileged huge-page/MSR availability.
