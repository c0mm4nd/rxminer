// SPDX-License-Identifier: BSD-3-Clause
// Compare AES rounds and complete scratchpad hash/refill state, including tails.
#include "aes_hash.hpp"
#include "soft_aes.h"
#include "randomx.h"
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <memory>
#include <vector>

template<bool soft> void rxm_legacy_hashAes1Rx4(const void*, size_t, void*);
template<bool soft> void rxm_legacy_fillAes1Rx4(void*, size_t, void*);
template<bool soft> void rxm_legacy_hashAndFillAes1Rx4(void*, size_t, void*, void*);
using Pipeline = void (*)(void*, size_t, void*, void*);
using Clock = std::chrono::steady_clock;

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

static double measure(Pipeline pipeline, Buffer& buffer, unsigned char* hash, unsigned char* state) {
    const auto start = Clock::now();
    pipeline(buffer.data, 2097152, hash, state);
    return std::chrono::duration<double, std::nano>(Clock::now() - start).count();
}

static void sample(bool hardware) {
    const auto deadline = Clock::now() + std::chrono::milliseconds(500);
    for (bool soft : {true, false}) {
        if ((!soft && !hardware) || Clock::now() >= deadline) break;
        Buffer input(2097152), working(2097152), expected(2097152);
        std::memset(input.data, 0, 2097152);
        alignas(64) unsigned char oldHash[64], newHash[64], oldState[64], newState[64];
        const auto before = soft ? rxm_legacy_hashAndFillAes1Rx4<true> : rxm_legacy_hashAndFillAes1Rx4<false>;
        const auto after = soft ? hashAndFillAes1Rx4<true> : hashAndFillAes1Rx4<false>;
        std::vector<double> oldTimes, newTimes;
        // Warmup plus four alternating pairs; compare the complete state outside timing.
        for (unsigned pair = 0; pair < 5 && Clock::now() < deadline; ++pair) {
            std::memset(oldState, 0, 64);
            std::memset(newState, 0, 64);
            input.data[0] = static_cast<unsigned char>(pair);
            // Both paths use the same address and identical copy-in warmup.
            // Separate buffers can bias results on CPUs with small/shared caches.
            const auto run = [&](Pipeline pipeline, unsigned char* hash, unsigned char* state) {
                std::memcpy(working.data, input.data, 2097152);
                return measure(pipeline, working, hash, state);
            };
            double a, b;
            if (pair % 2) {
                b = run(after, newHash, newState);
                std::memcpy(expected.data, working.data, 2097152);
                a = run(before, oldHash, oldState);
            } else {
                a = run(before, oldHash, oldState);
                std::memcpy(expected.data, working.data, 2097152);
                b = run(after, newHash, newState);
            }
            require(std::memcmp(oldHash, newHash, 64) == 0);
            require(std::memcmp(oldState, newState, 64) == 0);
            require(std::memcmp(expected.data, working.data, 2097152) == 0);
            if (pair) { oldTimes.push_back(a); newTimes.push_back(b); }
        }
        if (oldTimes.empty()) continue;
        std::sort(oldTimes.begin(), oldTimes.end());
        std::sort(newTimes.begin(), newTimes.end());
        const auto median = [](const std::vector<double>& times) {
            return (times[times.size() / 2] + times[(times.size() - 1) / 2]) / 2;
        };
        std::printf("RXM_AES_SAMPLE {\"mode\":\"%s\",\"pairs\":%u,\"legacy_ns\":%.2f,\"current_ns\":%.2f}\n",
            soft ? "software" : "hardware", unsigned(oldTimes.size()), median(oldTimes), median(newTimes));
    }
}

int main(int argc, char** argv) {
    require(argc == 1 || (argc == 2 && std::strcmp(argv[1], "--sample") == 0));
    const bool hardware = (randomx_get_flags() & RANDOMX_FLAG_HARD_AES) != 0;
    rounds();
    unsigned cases = check<true>();
    if (hardware) cases += check<false>();
    std::printf("PASSED AES hash/refill: %u full-buffer/state/tail cases\n", cases);
    if (argc == 2) sample(hardware);
}
