// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 RxMiner contributors
// Differential checks against the unchanged streaming API, including paths
// which must not take the register-file shortcut.
#include <cstddef>
#include "blake2/blake2.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <initializer_list>

static void require(bool value) { if (!value) std::abort(); }

static void reference(unsigned char* out, size_t outlen, const unsigned char* in,
                      size_t inlen, const unsigned char* key, size_t keylen) {
    blake2b_state state;
    require((keylen ? blake2b_init_key(&state, outlen, key, keylen)
                   : blake2b_init(&state, outlen)) == 0);
    // Split updates across block boundaries to exercise the streaming oracle.
    const size_t split = inlen / 3;
    require(blake2b_update(&state, in, split) == 0);
    require(blake2b_update(&state, in + split, inlen - split) == 0);
    require(blake2b_final(&state, out, outlen) == 0);
}

int main() {
    unsigned char input[520], key[64], expected[64], actual[80], overlap[600];
    unsigned cases = 0;
    for (unsigned sample = 0; sample < 32; ++sample) {
        uint64_t rng = 0x9192ed9bc331c785ULL + sample;
        for (auto& byte : input) {
            rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
            byte = static_cast<unsigned char>(rng);
        }
        for (size_t i = 0; i < sizeof(key); ++i) key[i] = input[i + 13];
        for (size_t size : {0u, 1u, 127u, 128u, 129u, 255u, 256u, 257u, 512u}) {
            for (size_t outlen : {1u, 31u, 32u, 33u, 63u, 64u}) {
                for (size_t keylen : {0u, 1u, 64u}) {
                    const unsigned offset = sample % 8;
                    reference(expected, outlen, input + offset, size, key, keylen);
                    std::memset(actual, 0xa5, sizeof(actual));
                    require(blake2b(actual + offset, outlen, input + offset, size, key, keylen) == 0);
                    require(std::memcmp(expected, actual + offset, outlen) == 0);
                    for (size_t i = 0; i < sizeof(actual); ++i)
                        if (i < offset || i >= offset + outlen) require(actual[i] == 0xa5);
                    ++cases;
                    if (size == 256 && keylen == 0 && (outlen == 32 || outlen == 64)) {
                        for (size_t destination : {0u, 1u, 127u, 240u}) {
                            std::memset(overlap, 0xa5, sizeof(overlap));
                            std::memcpy(overlap + 8, input + offset, size);
                            require(blake2b(overlap + destination, outlen, overlap + 8, size, nullptr, 0) == 0);
                            require(std::memcmp(expected, overlap + destination, outlen) == 0);
                            ++cases;
                        }
                    }
                }
            }
        }
    }
    require(blake2b(actual, 32, nullptr, 256, nullptr, 0) == -1);
    require(blake2b(nullptr, 32, input, 256, nullptr, 0) == -1);
    require(blake2b(actual, 0, input, 256, nullptr, 0) == -1);
    require(blake2b(actual, 65, input, 256, nullptr, 0) == -1);
    require(blake2b(actual, 32, input, 256, nullptr, 1) == -1);
    require(blake2b(actual, 32, input, 256, key, 65) == -1);
    std::printf("PASSED BLAKE2b differential: %u cases and 6 invalid inputs\n", cases);
}
