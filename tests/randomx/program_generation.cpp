// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 RxMiner contributors
// Compare the VM's active program with full-capacity generation across upgrades
// and downgrades. Check seed preservation and that inactive bytes are not written.
#include "virtual_machine.hpp"
#include "allocator.hpp"
#include "aes_hash.hpp"
#include <cstdio>
#include <cstdlib>
#include <cstring>

template<bool softAes>
class Probe : public randomx::VmBase<randomx::AlignedAllocator<randomx::CacheLineSize>, softAes> {
public:
    Probe() : randomx::VmBase<randomx::AlignedAllocator<randomx::CacheLineSize>, softAes>(RANDOMX_FLAG_DEFAULT) {}
    void run(void* seed) override { this->generateProgram(seed); }
    void runNormalized(void* seed) { this->generateProgramNormalized(seed); }
    void poison() { std::memset(&this->program, 0xa5, sizeof(this->program)); }
};

template<bool softAes>
static unsigned check() {
    Probe<softAes> vm;
    unsigned cases = 0;
    for (unsigned n = 0; n < 32; ++n) {
        alignas(64) unsigned char seed[64], saved[64];
        for (unsigned i = 0; i < sizeof(seed); ++i) seed[i] = (n * 37 + i * 13) ^ (i + n);
        std::memcpy(saved, seed, sizeof(seed));
        // AES generation uses aligned SIMD stores. Program's natural alignment
        // is only 8 bytes; match the VM's explicitly aligned program storage.
        alignas(64) randomx::Program reference{};
        fillAes4Rx4<softAes>(seed, sizeof(reference), &reference);
        for (bool v2 : {false, true, false, true}) {
            if (v2) vm.setFlagV2(); else vm.clearFlagV2();
            const size_t active = 128 + (v2 ? 384 : 256) * 8;
            for (bool normalized : {false, true}) {
                vm.poison();
                auto expected = reference;
                if (normalized) {
                    // Reference normalization is deliberately independent of
                    // the fused AES output mask, including the entropy boundary.
                    for (unsigned i = 0; i < (v2 ? 384u : 256u); ++i) {
                        expected(i).src %= 8;
                        expected(i).dst %= 8;
                    }
                    vm.runNormalized(seed);
                } else {
                    vm.run(seed);
                }
                if (std::memcmp(seed, saved, sizeof(seed)) ||
                    std::memcmp(&vm.getProgram(), &expected, active)) std::abort();
                const auto* bytes = reinterpret_cast<const unsigned char*>(&vm.getProgram());
                for (size_t i = active; i < sizeof(reference); ++i)
                    if (bytes[i] != 0xa5) std::abort();
                ++cases;
            }
        }
    }
    return cases;
}

int main() {
    unsigned cases = check<true>();
    if (randomx_get_flags() & RANDOMX_FLAG_HARD_AES) cases += check<false>();
    std::printf("PASSED program generation: %u prefix/seed/tail/switch cases\n", cases);
}
