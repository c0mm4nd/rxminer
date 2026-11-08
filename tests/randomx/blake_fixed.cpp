// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 RxMiner contributors
// Differential checks against the unchanged streaming API, including paths
// which must not take the one-shot shortcuts.
#include <cstddef>
#include "blake2/blake2.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <algorithm>
#include <chrono>
#include <utility>
#include <vector>

extern "C" int rxm_streaming_blake2b(void*, size_t, const void*, size_t, const void*, size_t);
using Hash = int (*)(void*, size_t, const void*, size_t, const void*, size_t);
using Clock = std::chrono::steady_clock;

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

struct Timing { double ns; unsigned checksum; };

static Timing measure(Hash hash, size_t inlen, size_t outlen) {
    unsigned char input[256] = {}, output[64];
    unsigned checksum = 0;
    constexpr unsigned count = 4096;
    const auto start = Clock::now();
    for (unsigned i = 0; i < count; ++i) {
        input[0] = static_cast<unsigned char>(i);
        require(hash(output, outlen, input, inlen, nullptr, 0) == 0);
        checksum += output[i % outlen];
    }
    return {std::chrono::duration<double, std::nano>(Clock::now() - start).count() / count, checksum};
}

static void sample() {
    const auto deadline = Clock::now() + std::chrono::milliseconds(500);
    for (const auto& shape : {std::pair<size_t, size_t>{64, 64}, {76, 64}, {128, 64}, {256, 32}}) {
        if (Clock::now() >= deadline) break;
        std::vector<double> streaming, current;
        require(measure(rxm_streaming_blake2b, shape.first, shape.second).checksum ==
                measure(blake2b, shape.first, shape.second).checksum);
        for (unsigned i = 0; i < 8 && Clock::now() < deadline; ++i) {
            Timing a, b;
            if (i % 2) {
                b = measure(blake2b, shape.first, shape.second);
                a = measure(rxm_streaming_blake2b, shape.first, shape.second);
            } else {
                a = measure(rxm_streaming_blake2b, shape.first, shape.second);
                b = measure(blake2b, shape.first, shape.second);
            }
            require(a.checksum == b.checksum);
            streaming.push_back(a.ns);
            current.push_back(b.ns);
        }
        if (streaming.empty()) break;
        std::sort(streaming.begin(), streaming.end());
        std::sort(current.begin(), current.end());
        const auto median = [](const std::vector<double>& values) {
            return (values[values.size() / 2] + values[(values.size() - 1) / 2]) / 2;
        };
        std::printf("RXM_ONESHOT_SAMPLE {\"input\":%u,\"output\":%u,\"pairs\":%u,\"streaming_ns\":%.2f,\"current_ns\":%.2f}\n",
            unsigned(shape.first), unsigned(shape.second), unsigned(streaming.size()), median(streaming), median(current));
    }
}

int main(int argc, char** argv) {
    require(argc == 1 || (argc == 2 && std::strcmp(argv[1], "--sample") == 0));
    unsigned char input[520], key[64], expected[64], actual[80], overlap[600];
    unsigned cases = 0;
    for (unsigned sample = 0; sample < 32; ++sample) {
        uint64_t rng = 0x9192ed9bc331c785ULL + sample;
        for (auto& byte : input) {
            rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
            byte = static_cast<unsigned char>(rng);
        }
        for (size_t i = 0; i < sizeof(key); ++i) key[i] = input[i + 13];
        std::vector<size_t> sizes;
        for (size_t size = 0; size <= 129; ++size) sizes.push_back(size);
        for (size_t size : {255u, 256u, 257u, 512u}) sizes.push_back(size);
        for (size_t size : sizes) {
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
                    if (size <= 256 && keylen == 0) {
                        for (size_t destination : {0u, 1u, 8u, 9u, 127u, 240u}) {
                            std::memset(overlap, 0xa5, sizeof(overlap));
                            std::memcpy(overlap + 8, input + offset, size);
                            require(blake2b(overlap + destination, outlen, overlap + 8, size, nullptr, 0) == 0);
                            require(std::memcmp(expected, overlap + destination, outlen) == 0);
                            for (size_t i = 0; i < sizeof(overlap); ++i) {
                                if (i >= destination && i < destination + outlen) continue;
                                require(overlap[i] == (i >= 8 && i < 8 + size ? input[offset + i - 8] : 0xa5));
                            }
                            ++cases;
                        }
                    }
                }
            }
        }
    }
    unsigned invalid = 0;
    for (size_t size : {0u, 1u, 64u, 128u, 129u, 256u}) {
        if (size != 0) {
            require(blake2b(actual, 32, nullptr, size, nullptr, 0) == -1);
            ++invalid;
        }
        require(blake2b(nullptr, 32, input, size, nullptr, 0) == -1);
        require(blake2b(actual, 0, input, size, nullptr, 0) == -1);
        require(blake2b(actual, 65, input, size, nullptr, 0) == -1);
        require(blake2b(actual, 32, input, size, nullptr, 1) == -1);
        require(blake2b(actual, 32, input, size, key, 65) == -1);
        invalid += 5;
    }
    for (size_t outlen : {32u, 64u}) {
        reference(expected, outlen, input, 0, nullptr, 0);
        require(blake2b(actual, outlen, nullptr, 0, nullptr, 0) == 0);
        require(std::memcmp(expected, actual, outlen) == 0);
    }
    std::printf("PASSED BLAKE2b differential: %u cases and %u invalid inputs\n", cases, invalid);
    if (argc == 2) sample();
}
