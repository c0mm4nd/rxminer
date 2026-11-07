# Platform support

CPU mining and Cargo installation are verified on the native CI targets below.
OS-specific tuning differences are listed separately.

| Target | Hash backend | Verified runner |
| --- | --- | --- |
| Linux x86-64 | Optimized x86-64 JIT | Ubuntu 22.04 and 24.04 |
| Linux ARM64 | Upstream ARM64 JIT | Ubuntu 24.04 ARM64 |
| macOS Intel | Optimized x86-64 JIT | macOS 15 Intel |
| macOS Apple Silicon | ARM64 JIT with write/execute protection | macOS 15 ARM64 |
| Windows x64 (MSVC) | Upstream Windows x64 JIT | Windows Server 2022 |

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

GPU mining and other algorithms are not part of this port. Windows ARM64 has
not been validated. See below for RISC-V validation. Release archives remain Linux x86-64 only;
other platforms use Cargo installation.

## RISC-V bring-up

The separate [RISC-V native regression workflow](../.github/workflows/riscv.yml)
uses RISE's `ubuntu-24.04-riscv` runner. The initial interpreter baseline passed
[native compilation, known hashes, CLI configuration, Cargo installation and light-mode mining](https://github.com/c0mm4nd/rxminer/actions/runs/42351301309).
The scalar RV64GC JIT and compiled dataset initializer are now integrated;
native JIT validation is pending. Vector/crypto acceleration is not integrated.

For a personal repository, install the
[RISE personal-account GitHub App](https://github.com/apps/rise-risc-v-runners-personal)
and grant access only to the intended repository. The service requires repository
Administration read/write to register ephemeral runners, plus Metadata read;
see the [provider's installation guide](https://riscv-runners.riseproject.dev/docs/getting-started/install.html).
Once installed, open Actions, select **RISC-V native regression**, and use
**Run workflow** on `main`. A maintainer can also run:

```bash
gh workflow run riscv.yml --ref main
```

The workflow checks native compilation, a known RandomX hash, pipelined hashing,
concurrent interpreter/JIT/secure-JIT agreement, shared-cache VM recreation,
full-dataset/light-mode agreement, CLI configuration, Cargo installation and
positive offline light-mode hash counts. It uses no pool connection. Relevant
pushes to `main` and pull requests trigger it automatically; manual dispatch
remains available. The existing platform matrix stays independent of the RISE
runner. Performance and instruction-extension coverage require further testing.

## Validation

The five verified platform targets above run Rust checks, known hash vectors, JIT/interpreter
agreement, full-memory and light-mode mining, and local Cargo installation.
Full-memory shares are checked against the light interpreter across concurrent
job updates, developer-fee switches and seed changes. Worker startup failures
and panics must stop the other workers and clear shared state.
The existing Linux x86-64 jobs additionally run the native differential suite,
local pool/proxy/runtime regressions and fresh Git-install checks.

The initial port passed all six jobs in the
[native CI run](https://github.com/c0mm4nd/rxminer/actions/runs/42283459720).

Hosted runners validate functional behavior. They do not establish comparable
mining performance or guarantee privileged huge-page/MSR availability.
