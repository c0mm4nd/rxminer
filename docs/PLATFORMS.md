# Platform support

The default `randomx-rust` feature selects Rust JITs on Linux x86-64/ARM64/RV64, macOS Intel/Apple Silicon,
and Windows x64. Each emitter implements VM instructions, superscalar code and
dataset initialization in Rust. The interpreter remains selectable on every
platform; other architectures default to it.

Linux x86-64 has native differential validation against the retained C++ library.
ARM64 and RV64 have local QEMU validation of instruction execution, complete
v1/v2 chains, full/light pipelines and seed replacement. Windows x64 and both
macOS architectures have cross-compilation checks of the RandomX modules and
miner adapter; they still need native OS runtime validation. CI workflows check
each backend feature separately. Consult the GitHub Actions result for the exact
commit; cross-compilation and historical C++ CI do not validate native execution
of the current Rust ports or the newly packaged native wrapper.

## Dependencies

Rust 1.88+, Cargo, a platform linker and a C toolchain for ring's TLS kernels are
required. The `randomx-rust` crate has no C/C++ build dependency. Selecting
`--no-default-features --features randomx-cpp` builds bundled native sources with
`cc` and also requires a C++ compiler. ring retains its separate transitive `cc`
dependency in both builds. OpenSSL is not linked.

- Linux: GCC or Clang (`gcc` on Debian/Ubuntu).
- macOS: Xcode Command Line Tools.
- Windows: Rust MSVC and Visual Studio build tools for the C toolchain/linker.

Default Rust builds need no C++ compiler. Neither backend needs CMake, Make,
Ninja or Python. Cross builds
still need the target tools required by ring and Rust linking. Install this
checkout with `cargo install --path . --locked`. For Git installs, select a
revision that contains the backend features you intend to build.

## Platform crate

`rx-platform` provides reusable CPU topology, worker selection, affinity,
priority, NUMA binding and CPU/memory availability queries. The miner keeps
RandomX-specific replication policy and diagnostic logging in its adapter.
MSR and battery management remain miner modules. Both RandomX backend features
use the same platform crate; it introduces no native build dependency.

## Current differences

Linux supports physical-core discovery, cgroup limits, NUMA placement, MSR
control and battery monitoring. Other platforms currently use a single dataset.

Windows discovers physical cores within the process's current processor group,
respects its affinity mask and binds automatic workers to those CPUs. SMT siblings
are used when requested. It
supports thread priorities, Ctrl+C/Ctrl+Break shutdown, logging and atomic
configuration replacement. Multi-group CPU discovery, NUMA placement, MSR
control, background mode and syslog are not implemented.

macOS uses unbound workers because it does not expose Linux-style hard CPU
affinity. Explicit CPU IDs and masks are rejected; use `-t N` or `cpu.rx`
entries of `-1`. Automatic worker count uses the OS physical-core count, with
logical processors available through `--smt`. Available-memory headroom is
estimated from free and inactive pages, allowing auto memory mode to choose a
light cache under memory pressure. Explicit fast mode retains the requested mode.

MSR tuning requires Linux x86-64. AES-NI is available on x86-64 and ARMv8 AES on
ARM64, including interpreter mode. SSSE3 and AVX2 Argon2 are runtime-selected on
x86-64; other architectures use scalar Argon2. Reserved page requests fall back
when unavailable. Linux implements 2 MiB/1 GiB mappings and explicit THP advice;
Windows and macOS implement their supported large-page allocation APIs.

The x86-64 JIT exposes all four scratchpad prefetch modes. Linux x86-64/ARM64
normally use separate RW/RX aliases, with a single-mapping secure option. RV64
uses a single RW/RX mapping plus process-wide instruction-cache synchronization
for reliable code replacement, including under QEMU. Windows uses VirtualProtect
and FlushInstructionCache. Apple Silicon uses MAP_JIT, pthread JIT write
protection and instruction-cache invalidation. Scratchpad pools are keyed by
NUMA node and actual page type on all platforms.

GPU mining, other algorithms and Windows ARM64 validation remain outside scope.

## Packaging and TLS

Local workflow definitions target Linux x86-64, ARM64 and RV64, macOS Intel and
Apple Silicon, and Windows x64. Release packaging currently selects the default
Rust feature: those Linux binaries must not link libstdc++, OpenSSL or libcrypto.
C++ feature builds may link the platform C++ runtime. Windows packaging retains the Visual Studio runtime bundle until
its dependencies can be checked on Windows. macOS packages are ad-hoc signed.
None of these updated migration packages has been published.

TLS uses rustls with ring and platform roots from rustls-native-certs.
`SSL_CERT_FILE` and `SSL_CERT_DIR` override native root loading; `--tls-ca`
adds a PEM bundle. `--tls-fingerprint` is enforced even when CA verification is
also enabled. The transitive `openssl-probe` crate discovers certificate paths;
it does not link OpenSSL.

## Historical native validation

The C++ backend passed the [six-job native CI run](https://github.com/c0mm4nd/rxminer/actions/runs/42283459720).
The RV64GC JIT passed the [RISC-V validation batch](https://github.com/c0mm4nd/rxminer/actions/runs/42358525166).
Those results cover the previous implementation. C++ remains only as an optional
Linux x86-64 reference build in the full checkout; see [development](DEVELOPMENT.md).

The RISC-V workflow still queues only on explicit dispatch or a release batch.
Do not dispatch migration workflows or publish migration commits without a new
publication request. Hosted-runner checks establish functional behavior, not
comparable mining performance or privileged page/MSR availability.
