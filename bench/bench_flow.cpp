// SPDX-License-Identifier: BSD-3-Clause
// Stage timings for the actual RandomX pipeline, checked against single hashes.
// Usage: bench_flow [jit|interpreter] [light|full] [v1|v2] [hashes:1..64] [default|secure]
#include "randomx.h"
#include "virtual_machine.hpp"
#include "vm_compiled.hpp"
#include "blake2/blake2.h"
#include <sched.h>
#include <array>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <thread>
#include <vector>

using Clock = std::chrono::steady_clock;
static double elapsed(Clock::time_point start) {
    return std::chrono::duration<double, std::milli>(Clock::now() - start).count();
}
static void require(bool value) { if (!value) std::abort(); }

// Single-threaded benchmark instrumentation only; the production VM and its
// layout stay unchanged. Reference hashes call the original implementation on
// this very same VM, so they check the reconstructed phase sequence below.
static struct {
    bool enabled = false;
    unsigned programs = 0;
    double generate = 0, initialize = 0, write = 0, compile = 0, execute = 0, protect = 0;
} stages;

template<bool softAes, bool secure>
class ProfiledVm : public randomx::CompiledVm<randomx::AlignedAllocator<randomx::CacheLineSize>, softAes, secure> {
    using Base = randomx::CompiledVm<randomx::AlignedAllocator<randomx::CacheLineSize>, softAes, secure>;
public:
    explicit ProfiledVm(randomx_flags flags) : Base(flags) {}
    void run(void* seed) override {
        if (!stages.enabled) { Base::run(seed); return; }
        auto start = Clock::now();
#if defined(RXM_NORMALIZED_PROGRAM)
        this->generateProgramNormalized(seed);
#else
        this->generateProgram(seed);
#endif
        stages.generate += elapsed(start);
        start = Clock::now();
        this->initialize();
        stages.initialize += elapsed(start);
        if (secure) {
            start = Clock::now();
            this->compiler.enableWriting();
            stages.write += elapsed(start);
        }
        start = Clock::now();
#if defined(RXM_NORMALIZED_PROGRAM)
        this->compiler.generateProgram(this->program, this->config, true);
#else
        this->compiler.generateProgram(this->program, this->config);
#endif
        stages.compile += elapsed(start);
        if (secure) {
            start = Clock::now();
            this->compiler.enableExecution();
            stages.protect += elapsed(start);
        }
        this->mem.memory = this->datasetPtr->memory + this->datasetOffset;
        start = Clock::now();
        this->execute();
        stages.execute += elapsed(start);
        ++stages.programs;
    }
};

template<bool softAes, bool secure>
static randomx_vm* profiledVm(randomx_flags flags, randomx_dataset* dataset) {
    using VM = ProfiledVm<softAes, secure>;
    using Base = randomx::CompiledVm<randomx::AlignedAllocator<randomx::CacheLineSize>, softAes, secure>;
    // The inherited delete operator frees sizeof(Base), so do not add fields.
    static_assert(sizeof(VM) == sizeof(Base), "profile VM must not add storage");
    std::unique_ptr<VM> vm(new VM(flags));
    vm->setDataset(dataset);
    vm->allocate();
    return vm.release();
}
static std::array<uint8_t, 76> input(unsigned nonce) {
    std::array<uint8_t, 76> bytes{};
    for (unsigned i = 0; i < bytes.size(); ++i) bytes[i] = static_cast<uint8_t>(i * 17);
    bytes[39] = static_cast<uint8_t>(nonce);
    return bytes;
}

int main(int argc, char** argv) {
    if (argc > 6 || (argc > 1 && std::strcmp(argv[1], "jit") && std::strcmp(argv[1], "interpreter")) ||
        (argc > 2 && std::strcmp(argv[2], "light") && std::strcmp(argv[2], "full")) ||
        (argc > 3 && std::strcmp(argv[3], "v1") && std::strcmp(argv[3], "v2")) ||
        (argc > 5 && std::strcmp(argv[5], "default") && std::strcmp(argv[5], "secure"))) {
        std::fputs("Usage: bench_flow [jit|interpreter] [light|full] [v1|v2] [hashes:1..64] [default|secure]\n", stderr);
        return 1;
    }
    char* end = nullptr;
    const auto count = argc > 4 ? std::strtoul(argv[4], &end, 10) : 8;
    if (count < 1 || count > 64 || (end && *end)) return 1;
    const bool jit = argc < 2 || std::strcmp(argv[1], "jit") == 0;
    const bool full = argc > 2 && std::strcmp(argv[2], "full") == 0;
    const bool v2 = argc > 3 && std::strcmp(argv[3], "v2") == 0;
    const bool secure = argc > 5 && std::strcmp(argv[5], "secure") == 0;
    if (secure && !jit) return 1;
    const auto supported = randomx_get_flags();
    if (jit && !(supported & RANDOMX_FLAG_JIT)) {
        std::fputs("JIT unavailable on this target\n", stderr);
        return 1;
    }
    using Cache = std::unique_ptr<randomx_cache, decltype(&randomx_release_cache)>;
    using Dataset = std::unique_ptr<randomx_dataset, decltype(&randomx_release_dataset)>;
    using Vm = std::unique_ptr<randomx_vm, decltype(&randomx_destroy_vm)>;
    Cache cache(randomx_alloc_cache(static_cast<randomx_flags>(supported &
        (RANDOMX_FLAG_JIT | RANDOMX_FLAG_ARGON2))), randomx_release_cache);
    require(bool(cache));
    constexpr char cacheKey[] = "RxMiner-flow-probe";
    randomx_init_cache(cache.get(), cacheKey, sizeof(cacheKey) - 1);
    Dataset dataset(nullptr, randomx_release_dataset);
    if (full) {
        dataset.reset(randomx_alloc_dataset(RANDOMX_FLAG_DEFAULT));
        require(bool(dataset));
        const auto items = randomx_dataset_item_count();
        std::vector<std::thread> workers;
        for (unsigned i = 0; i < 4; ++i) workers.emplace_back([&, i] {
            const auto begin = items * i / 4;
            const auto end = items * (i + 1) / 4;
            randomx_init_dataset(dataset.get(), cache.get(), begin, end - begin);
        });
        for (auto& worker : workers) worker.join();
    }
    // Keep initialization parallel, then pin hashing to the first allowed CPU.
    cpu_set_t allowed, selected;
    CPU_ZERO(&allowed); CPU_ZERO(&selected);
    require(sched_getaffinity(0, sizeof(allowed), &allowed) == 0);
    int cpu = 0;
    while (cpu < CPU_SETSIZE && !CPU_ISSET(cpu, &allowed)) ++cpu;
    require(cpu < CPU_SETSIZE);
    CPU_SET(cpu, &selected);
    require(sched_setaffinity(0, sizeof(selected), &selected) == 0);
    const auto flags = static_cast<randomx_flags>((supported & RANDOMX_FLAG_HARD_AES) |
        (jit ? (supported & (RANDOMX_FLAG_JIT | RANDOMX_FLAG_SECURE)) : 0) |
        (full ? RANDOMX_FLAG_FULL_MEM : 0) | (v2 ? RANDOMX_FLAG_V2 : 0) |
        (secure ? RANDOMX_FLAG_SECURE : 0));
    randomx_vm* machine;
    if (jit && full) {
        if (flags & RANDOMX_FLAG_HARD_AES)
            machine = flags & RANDOMX_FLAG_SECURE ? profiledVm<false, true>(flags, dataset.get()) : profiledVm<false, false>(flags, dataset.get());
        else
            machine = flags & RANDOMX_FLAG_SECURE ? profiledVm<true, true>(flags, dataset.get()) : profiledVm<true, false>(flags, dataset.get());
    } else machine = randomx_create_vm(flags, full ? nullptr : cache.get(), dataset.get());
    Vm vm(machine, randomx_destroy_vm);
    require(bool(vm));
    std::vector<std::array<uint8_t, 32>> expected(count);
    auto bytes = input(0);
    randomx_calculate_hash(vm.get(), bytes.data(), bytes.size(), expected[0].data());
    const auto referenceStart = Clock::now();
    for (unsigned i = 0; i < count; ++i) {
        bytes = input(i);
        randomx_calculate_hash(vm.get(), bytes.data(), bytes.size(), expected[i].data());
    }
    const double referenceMs = elapsed(referenceStart);

    double seedMs = 0, vmMs = 0, chainMs = 0, fusedMs = 0, drainMs = 0;
    alignas(64) uint64_t seed[8];
    std::array<uint8_t, 32> actual;
    stages.enabled = jit && full;
    const auto pipelineStart = Clock::now();
    bytes = input(0);
    auto start = Clock::now();
    require(blake2b(seed, sizeof(seed), bytes.data(), bytes.size(), nullptr, 0) == 0);
    seedMs += elapsed(start);
    start = Clock::now();
    vm->initScratchpad(seed);
    const double fillMs = elapsed(start);
    for (unsigned i = 0; i < count; ++i) {
        vm->resetRoundingMode();
        for (unsigned chain = 0; chain < RANDOMX_PROGRAM_COUNT; ++chain) {
            start = Clock::now();
            vm->run(seed);
            vmMs += elapsed(start);
            if (chain + 1 < RANDOMX_PROGRAM_COUNT) {
                start = Clock::now();
                require(blake2b(seed, sizeof(seed), vm->getRegisterFile(), sizeof(randomx::RegisterFile), nullptr, 0) == 0);
                chainMs += elapsed(start);
            }
        }
        if (i + 1 < count) {
            bytes = input(i + 1);
            start = Clock::now();
            require(blake2b(seed, sizeof(seed), bytes.data(), bytes.size(), nullptr, 0) == 0);
            seedMs += elapsed(start);
            start = Clock::now();
            vm->hashAndFill(actual.data(), actual.size(), seed);
            fusedMs += elapsed(start);
        } else {
            start = Clock::now();
            vm->getFinalResult(actual.data(), actual.size());
            drainMs += elapsed(start);
        }
        require(actual == expected[i]);
    }
    const double pipelineMs = elapsed(pipelineStart);
    stages.enabled = false;
    vm->resetRoundingMode();
    std::printf("%s %s v%u: %lu hashes verified (initialization/warmup excluded)\n",
        jit ? "JIT" : "interpreter", full ? "full" : "light", v2 ? 2 : 1, count);
    std::printf("single-hash API: %.3f ms/hash; instrumented pipeline: %.3f ms/hash\n",
        referenceMs / count, pipelineMs / count);
    std::printf("Stage totals (ms): input BLAKE2b=%.3f, initial fill=%.3f, VM chains=%.3f,\n"
        "  chain BLAKE2b=%.3f, fused finish/refill=%.3f, final drain=%.3f\n",
        seedMs, fillMs, vmMs, chainMs, fusedMs, drainMs);
    std::printf("VM chains: %.2f%% of instrumented wall time (includes generation/compilation).\n",
        100 * vmMs / pipelineMs);
    if (jit && full) {
        require(stages.programs == count * RANDOMX_PROGRAM_COUNT);
        std::printf("VM detail (ms, cpu=%d, secure=%u, programs=%u): generation=%.3f, initialization=%.3f,\n"
            "  writable=%.3f, compilation=%.3f, executable=%.3f, execution=%.3f\n",
            cpu, bool(flags & RANDOMX_FLAG_SECURE), stages.programs, stages.generate, stages.initialize,
            stages.write, stages.compile, stages.protect, stages.execute);
        std::printf("JIT execution: %.2f%% of instrumented pipeline time; phase timers add overhead.\n",
            100 * stages.execute / pipelineMs);
    }
}
