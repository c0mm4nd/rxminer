// SPDX-License-Identifier: BSD-3-Clause
// Correctness against the compact-round implementation; optional bounded sampling.
#include <cstddef>
#include "blake2/blake2.h"
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <utility>
#include <vector>

extern "C" int rxm_compact_blake2b(void*, size_t, const void*, size_t, const void*, size_t);
extern "C" int rxm_compact_long(void*, size_t, const void*, size_t);
using Hash = int (*)(void*, size_t, const void*, size_t, const void*, size_t);
using Clock = std::chrono::steady_clock;

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
    // At most three input shapes, four paired samples each, with a shared 500 ms
    // budget checked between batches. This measures preparation, never mining.
    const auto deadline = Clock::now() + std::chrono::milliseconds(500);
    for (const auto& shape : {std::pair<size_t, size_t>{64, 64}, {256, 64}, {256, 32}}) {
        if (Clock::now() >= deadline) break;
        std::vector<double> compact, optimized;
        require(measure(rxm_compact_blake2b, shape.first, shape.second).checksum ==
                measure(blake2b, shape.first, shape.second).checksum);
        for (unsigned i = 0; i < 4 && Clock::now() < deadline; ++i) {
            Timing a, b;
            if (i % 2) {
                b = measure(blake2b, shape.first, shape.second);
                a = measure(rxm_compact_blake2b, shape.first, shape.second);
            } else {
                a = measure(rxm_compact_blake2b, shape.first, shape.second);
                b = measure(blake2b, shape.first, shape.second);
            }
            require(a.checksum == b.checksum);
            compact.push_back(a.ns);
            optimized.push_back(b.ns);
        }
        if (compact.empty()) break;
        std::sort(compact.begin(), compact.end());
        std::sort(optimized.begin(), optimized.end());
        const auto middle = compact.size() / 2;
        const auto median = [middle](const std::vector<double>& values) {
            return (values[middle] + values[(values.size() - 1) / 2]) / 2;
        };
        std::printf("RXM_SHORT_SAMPLE {\"input\":%u,\"output\":%u,\"pairs\":%u,\"compact_ns\":%.2f,\"optimized_ns\":%.2f}\n",
            unsigned(shape.first), unsigned(shape.second), unsigned(compact.size()), median(compact), median(optimized));
    }
}

int main(int argc, char** argv) {
    require(argc == 1 || (argc == 2 && std::strcmp(argv[1], "--sample") == 0));
    check();
    if (argc == 2) sample();
}
