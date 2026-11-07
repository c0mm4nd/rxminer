// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 RxMiner contributors
// Compare machine code from raw and fused-normalized programs on each backend.
#include "jit_compiler.hpp"
#include "program.hpp"
#include "aes_hash.hpp"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

int main() {
#if defined(RXM_NORMALIZED_PROGRAM)
    randomx::JitCompiler jit;
    unsigned cases = 0;
    for (unsigned n = 0; n < 32; ++n) {
        alignas(64) unsigned char seed[64];
        for (unsigned i = 0; i < sizeof(seed); ++i) seed[i] = (n * 37 + i * 13) ^ (i + n);
        // Reuse the allocation across v1/v2, hard/soft AES and memory-mode changes.
        for (bool v2 : {false, true, false, true}) {
            for (bool hardAes : {false, true}) {
                const auto flags = static_cast<randomx_flags>(RANDOMX_FLAG_JIT |
                    (v2 ? RANDOMX_FLAG_V2 : 0) | (hardAes ? RANDOMX_FLAG_HARD_AES : 0));
                jit.setFlags(flags);
                for (bool light : {false, true}) {
                    alignas(64) randomx::Program raw{}, normalized{};
                    const auto size = randomx::Program::getSizeBytes(flags);
                    fillAes4Rx4<true>(seed, size, &raw);
                    fillAes4Rx4Program<true>(seed, size, &normalized);
                    randomx::ProgramConfiguration config{};
                    config.eMask[0] = config.eMask[1] = 0x3ff0000000000000ULL;
                    config.readReg0 = n & 1;
                    config.readReg1 = 2 + (n & 1);
                    config.readReg2 = 4 + (n & 1);
                    config.readReg3 = 6 + (n & 1);
                    jit.enableWriting();
                    if (light) jit.generateProgramLight(raw, config, n * 64);
                    else jit.generateProgram(raw, config);
                    jit.enableExecution();
                    const std::vector<uint8_t> expected(jit.getCode(), jit.getCode() + jit.getCodeSize());
                    jit.enableWriting();
                    if (light) jit.generateProgramLight(normalized, config, n * 64, true);
                    else jit.generateProgram(normalized, config, true);
                    jit.enableExecution();
                    if (std::memcmp(expected.data(), jit.getCode(), expected.size())) {
                        std::fprintf(stderr, "JIT mismatch: seed=%u v2=%d hardAES=%d light=%d\n",
                            n, v2, hardAes, light);
                        std::abort();
                    }
                    ++cases;
                }
            }
        }
    }
    std::printf("PASSED raw/fused JIT machine code: %u cases\n", cases);
#else
    std::puts("SKIPPED fused JIT normalization: not used by this backend");
#endif
}
