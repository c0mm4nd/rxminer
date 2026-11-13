// SPDX-License-Identifier: BSD-3-Clause
// Correctness against the compact-round implementation.
#include <cstddef>
#include "blake2/blake2.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <initializer_list>

extern "C" int rxm_compact_blake2b(void*, size_t, const void*, size_t, const void*, size_t);
extern "C" int rxm_compact_long(void*, size_t, const void*, size_t);

static void require(bool value) { if (!value) std::abort(); }

static void check() {
    unsigned char input[1032], expected[1024], actual[1024];
    unsigned cases = 0;
    for (unsigned sample = 0; sample < 16; ++sample) {
        uint64_t state = 0x9192ed9bc331c785ULL + sample;
        for (auto& byte : input) {
            state ^= state << 13; state ^= state >> 7; state ^= state << 17;
            byte = static_cast<unsigned char>(state);
        }
        for (size_t size : {0u, 1u, 64u, 127u, 128u, 129u, 255u, 256u, 257u, 1024u}) {
            for (size_t outlen : {1u, 32u, 63u, 64u}) {
                for (size_t keylen : {0u, 1u, 64u}) {
                    const auto* data = input + sample % 8;
                    require(rxm_compact_blake2b(expected, outlen, data, size, input, keylen) == 0);
                    require(blake2b(actual, outlen, data, size, input, keylen) == 0);
                    require(std::memcmp(actual, expected, outlen) == 0);
                    ++cases;
                }
            }
        }
        for (size_t outlen : {1u, 32u, 64u, 65u, 128u, 1024u}) {
            require(rxm_compact_long(expected, outlen, input, 256) == 0);
            require(blake2b_long(actual, outlen, input, 256) == 0);
            require(std::memcmp(actual, expected, outlen) == 0);
            ++cases;
        }
    }
    std::printf("PASSED compact/unrolled BLAKE2b differential: %u cases\n", cases);
}

int main() {
    check();
}
