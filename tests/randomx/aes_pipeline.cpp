// SPDX-License-Identifier: BSD-3-Clause
// Compare AES rounds and complete scratchpad hash/refill state, including tails.
#include "aes_hash.hpp"
#include "soft_aes.h"
#include "randomx.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <memory>

template<bool soft> void rxm_legacy_hashAes1Rx4(const void*, size_t, void*);
template<bool soft> void rxm_legacy_fillAes1Rx4(void*, size_t, void*);
template<bool soft> void rxm_legacy_hashAndFillAes1Rx4(void*, size_t, void*, void*);

static void require(bool value) { if (!value) std::abort(); }
struct Buffer {
    std::unique_ptr<unsigned char[]> allocation;
    unsigned char* data;
    explicit Buffer(size_t size) : allocation(new unsigned char[size + 63]),
        data(reinterpret_cast<unsigned char*>((reinterpret_cast<uintptr_t>(allocation.get()) + 63) & ~uintptr_t(63))) {}
};

static void rounds() {
    uint64_t rng = 0x1c42b1ac1fc4a929ULL;
    for (unsigned n = 0; n < 4096; ++n) {
        alignas(64) unsigned char in[16], key[16], expected[16], actual[16];
        for (unsigned i = 0; i < 32; ++i) {
            rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
            (i < 16 ? in[i] : key[i - 16]) = static_cast<unsigned char>(rng);
        }
        const auto input = rx_load_vec_i128(reinterpret_cast<rx_vec_i128*>(in));
        const auto roundKey = rx_load_vec_i128(reinterpret_cast<rx_vec_i128*>(key));
        rx_store_vec_i128(reinterpret_cast<rx_vec_i128*>(expected), soft_aesenc(input, roundKey));
        rx_store_vec_i128(reinterpret_cast<rx_vec_i128*>(actual), aesenc<true>(input, roundKey));
        require(std::memcmp(expected, actual, 16) == 0);
        rx_store_vec_i128(reinterpret_cast<rx_vec_i128*>(expected), soft_aesdec(input, roundKey));
        rx_store_vec_i128(reinterpret_cast<rx_vec_i128*>(actual), aesdec<true>(input, roundKey));
        require(std::memcmp(expected, actual, 16) == 0);
    }
    std::puts("PASSED software AES round equivalence: 8192 cases");
}

template<bool soft>
static unsigned check() {
    unsigned cases = 0;
    for (size_t size : {4096u, 4160u, 4224u, 8192u, 2097152u}) {
        Buffer expected(size), actual(size), legacy(size);
        for (unsigned seed = 0; seed < (size == 2097152 ? 1u : 4u); ++seed) {
            alignas(64) unsigned char state[64], expectedState[64], legacyState[64];
            alignas(64) unsigned char hash[64], expectedHash[64], legacyHash[64];
            for (unsigned i = 0; i < 64; ++i) state[i] = (i * 37 + seed) ^ (i + seed * 13);
            std::memcpy(expectedState, state, 64);
            std::memcpy(legacyState, state, 64);
            for (size_t i = 0; i < size; ++i) expected.data[i] = (i * 17 + seed) ^ (i >> 11);
            std::memcpy(actual.data, expected.data, size);
            std::memcpy(legacy.data, expected.data, size);
            // Independent two-pass reference checks reads happen before stores.
            rxm_legacy_hashAes1Rx4<soft>(expected.data, size, expectedHash);
            rxm_legacy_fillAes1Rx4<soft>(expectedState, size, expected.data);
            rxm_legacy_hashAndFillAes1Rx4<soft>(legacy.data, size, legacyHash, legacyState);
            hashAndFillAes1Rx4<soft>(actual.data, size, hash, state);
            require(std::memcmp(hash, expectedHash, 64) == 0);
            require(std::memcmp(state, expectedState, 64) == 0);
            require(std::memcmp(actual.data, expected.data, size) == 0);
            require(std::memcmp(legacyHash, expectedHash, 64) == 0);
            require(std::memcmp(legacyState, expectedState, 64) == 0);
            require(std::memcmp(legacy.data, expected.data, size) == 0);
            ++cases;
        }
    }
    return cases;
}

int main() {
    const bool hardware = (randomx_get_flags() & RANDOMX_FLAG_HARD_AES) != 0;
    rounds();
    unsigned cases = check<true>();
    if (hardware) cases += check<false>();
    std::printf("PASSED AES hash/refill: %u full-buffer/state/tail cases\n", cases);
}
