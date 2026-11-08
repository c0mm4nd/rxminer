// SPDX-License-Identifier: BSD-3-Clause
// Linux paired RandomX benchmark. Both libraries must use the same VM-base and dataset ABI.
// Build each .so with -shared -Wl,-Bsymbolic -Wl,--whole-archive LIBRARY.a
// -Wl,--no-whole-archive -pthread -lm so internal references stay in that build.
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "randomx.h"
#include "virtual_machine.hpp"
#include <dlfcn.h>
#include <sched.h>
#include <sys/mman.h>
#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <thread>
#include <time.h>
#include <vector>

static void require(bool value) { if (!value) std::abort(); }
struct Library {
    void* handle;
#define API(name) decltype(&randomx_##name) name
    API(get_flags); API(alloc_cache); API(init_cache); API(release_cache);
    API(alloc_dataset); API(dataset_item_count); API(init_dataset); API(release_dataset);
    API(create_vm); API(destroy_vm);
    API(calculate_hash_first); API(calculate_hash_next); API(calculate_hash_last);
#undef API
    explicit Library(const char* path) : handle(dlopen(path, RTLD_NOW | RTLD_LOCAL)) {
        if (!handle) { std::fprintf(stderr, "%s\n", dlerror()); std::exit(1); }
#define LOAD(name) name = reinterpret_cast<decltype(name)>(dlsym(handle, "randomx_" #name)); require(name != nullptr)
        LOAD(get_flags); LOAD(alloc_cache); LOAD(init_cache); LOAD(release_cache);
        LOAD(alloc_dataset); LOAD(dataset_item_count); LOAD(init_dataset); LOAD(release_dataset);
        LOAD(create_vm); LOAD(destroy_vm);
        LOAD(calculate_hash_first); LOAD(calculate_hash_next); LOAD(calculate_hash_last);
#undef LOAD
    }
    ~Library() { dlclose(handle); }
};
using Hash = std::array<unsigned char, 32>;
using Clock = std::chrono::steady_clock;
struct Timing { double wall, cpu; };
// Exchange physical scratchpad mappings outside timing while keeping both VM
// pointers unchanged. The next batch refills every byte before execution.
// Both VMs use ordinary 2 MiB mappings and the same base-class/dataset ABI.
static void exchangeScratchpads(randomx_vm* first, randomx_vm* second) {
    constexpr size_t size = randomx::ScratchpadSize;
    void* a = const_cast<void*>(first->getScratchpad());
    void* b = const_cast<void*>(second->getScratchpad());
    require(a != b && (reinterpret_cast<uintptr_t>(a) % size) == 0 &&
        (reinterpret_cast<uintptr_t>(b) % size) == 0);
    void* reservation = mmap(nullptr, 2 * size, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    require(reservation != MAP_FAILED);
    const auto base = reinterpret_cast<uintptr_t>(reservation);
    const auto aligned = (base + size - 1) & ~(uintptr_t(size) - 1);
    const auto prefix = aligned - base;
    if (prefix) require(munmap(reservation, prefix) == 0);
    require(munmap(reinterpret_cast<void*>(aligned + size), size - prefix) == 0);
    void* temporary = reinterpret_cast<void*>(aligned);
    require(mremap(a, size, size, MREMAP_MAYMOVE | MREMAP_FIXED, temporary) == temporary);
    require(mremap(b, size, size, MREMAP_MAYMOVE | MREMAP_FIXED, a) == a);
    require(mremap(temporary, size, size, MREMAP_MAYMOVE | MREMAP_FIXED, b) == b);
}

static Timing run(Library& library, randomx_vm* vm, unsigned batch, std::vector<Hash>& hashes) {
    unsigned char input[76];
    for (unsigned i = 0; i < sizeof(input); ++i) input[i] = static_cast<unsigned char>(i * 17 + batch);
    input[39] = 0;
    input[40] = 0;
    timespec cpuStart, cpuEnd;
    const auto start = Clock::now();
    require(clock_gettime(CLOCK_THREAD_CPUTIME_ID, &cpuStart) == 0);
    library.calculate_hash_first(vm, input, sizeof(input));
    for (unsigned i = 1; i < hashes.size(); ++i) {
        input[39] = static_cast<unsigned char>(i);
        input[40] = static_cast<unsigned char>(i >> 8);
        library.calculate_hash_next(vm, input, sizeof(input), hashes[i - 1].data());
    }
    library.calculate_hash_last(vm, hashes.back().data());
    require(clock_gettime(CLOCK_THREAD_CPUTIME_ID, &cpuEnd) == 0);
    const double cpuMs = (cpuEnd.tv_sec - cpuStart.tv_sec) * 1000.0 +
        (cpuEnd.tv_nsec - cpuStart.tv_nsec) / 1e6;
    return {std::chrono::duration<double, std::milli>(Clock::now() - start).count() / hashes.size(),
        cpuMs / hashes.size()};
}
int main(int argc, char** argv) {
    if (argc < 3 || argc > 6) {
        std::fprintf(stderr, "Usage: bench_compare BEFORE.so AFTER.so [pairs:1..32] [hashes:1..1024] [v1|v2]\n");
        return 1;
    }
    const auto number = [](const char* text, unsigned limit) {
        char* end;
        const auto value = std::strtoul(text, &end, 10);
        require(*end == 0 && value > 0 && value <= limit);
        return static_cast<unsigned>(value);
    };
    const auto pairs = argc > 3 ? number(argv[3], 32) : 8;
    const auto count = argc > 4 ? number(argv[4], 1024) : 64;
    require(argc < 6 || !std::strcmp(argv[5], "v1") || !std::strcmp(argv[5], "v2"));
    const bool v2 = argc > 5 && !std::strcmp(argv[5], "v2");
    Library before(argv[1]), after(argv[2]);
    if (before.handle == after.handle)
        std::puts("Self-control: identical code with separate VM allocations.");
    std::puts("Physical scratchpads cross over between pairs; JIT/VM addresses remain distinct. Run reverse-order and self-controls.");
    require(before.dataset_item_count() == after.dataset_item_count());
    auto* cache = before.alloc_cache(static_cast<randomx_flags>(before.get_flags() & (RANDOMX_FLAG_JIT | RANDOMX_FLAG_ARGON2)));
    require(cache != nullptr);
    constexpr char key[] = "RxMiner paired JIT benchmark";
    before.init_cache(cache, key, sizeof(key) - 1);
    auto* dataset = before.alloc_dataset(RANDOMX_FLAG_DEFAULT);
    require(dataset != nullptr);
    const auto items = before.dataset_item_count();
    std::vector<std::thread> workers;
    for (unsigned i = 0; i < 4; ++i) workers.emplace_back([&, i] {
        const auto begin = items * i / 4, end = items * (i + 1) / 4;
        before.init_dataset(dataset, cache, begin, end - begin);
    });
    for (auto& thread : workers) thread.join();
    before.release_cache(cache);
    // Pin after initialization so four initialization workers can run in parallel.
    cpu_set_t allowed, selected;
    CPU_ZERO(&allowed); CPU_ZERO(&selected);
    require(sched_getaffinity(0, sizeof(allowed), &allowed) == 0);
    int cpu = 0;
    while (cpu < CPU_SETSIZE && !CPU_ISSET(cpu, &allowed)) ++cpu;
    require(cpu < CPU_SETSIZE);
    CPU_SET(cpu, &selected);
    require(sched_setaffinity(0, sizeof(selected), &selected) == 0);
    const auto common = before.get_flags() & after.get_flags();
    require((common & RANDOMX_FLAG_JIT) != 0);
    const auto flags = static_cast<randomx_flags>(RANDOMX_FLAG_JIT | RANDOMX_FLAG_FULL_MEM |
        (common & RANDOMX_FLAG_HARD_AES) | (v2 ? RANDOMX_FLAG_V2 : 0));
    randomx_vm* vm[2] = {before.create_vm(flags, nullptr, dataset), after.create_vm(flags, nullptr, dataset)};
    require(vm[0] && vm[1]);
    std::vector<Hash> expected(count), actual(count);
    std::vector<double> ratios, oldTimes, newTimes, cpuRatios, oldCpu, newCpu;
    for (unsigned pair = 0; pair < pairs + 4; ++pair) {
        Timing a, b;
        // Four combinations: both execution orders with both physical mappings.
        // Warmup uses the same four-way schedule as the measured samples.
        if ((pair / 2) % 2) {
            b = run(after, vm[1], pair, actual);
            a = run(before, vm[0], pair, expected);
        } else {
            a = run(before, vm[0], pair, expected);
            b = run(after, vm[1], pair, actual);
        }
        require(expected == actual);
        if (pair >= 4) {
            ratios.push_back(a.wall / b.wall); oldTimes.push_back(a.wall); newTimes.push_back(b.wall);
            cpuRatios.push_back(a.cpu / b.cpu); oldCpu.push_back(a.cpu); newCpu.push_back(b.cpu);
            std::printf("pair=%u before_ms=%.4f after_ms=%.4f ratio=%.4f before_cpu_ms=%.4f after_cpu_ms=%.4f cpu_ratio=%.4f\n",
                pair - 3, a.wall, b.wall, a.wall / b.wall, a.cpu, b.cpu, a.cpu / b.cpu);
            std::fflush(stdout);
        }
        exchangeScratchpads(vm[0], vm[1]);
    }
    const auto median = [](std::vector<double>& values) {
        std::sort(values.begin(), values.end());
        return (values[values.size()/2] + values[(values.size()-1)/2])/2;
    };
    // Ratios are multiplicative: equal slow/fast mapping effects must cancel.
    // An arithmetic midpoint of reciprocal ratios (e.g. 2 and 0.5) is biased.
    const auto geometricMean = [](const std::vector<double>& values) {
        double sum = 0;
        for (double value : values) sum += std::log(value);
        return std::exp(sum / values.size());
    };
    std::printf("cpu=%d version=%u hashes_per_batch=%u pairs=%u before_ms=%.4f after_ms=%.4f geometric_mean_pair_ratio=%.4f before_cpu_ms=%.4f after_cpu_ms=%.4f geometric_mean_cpu_ratio=%.4f all_hashes_equal\n",
        cpu, v2 ? 2 : 1, count, pairs, median(oldTimes), median(newTimes), geometricMean(ratios), median(oldCpu), median(newCpu), geometricMean(cpuRatios));
    before.destroy_vm(vm[0]); after.destroy_vm(vm[1]);
    before.release_dataset(dataset);
}
