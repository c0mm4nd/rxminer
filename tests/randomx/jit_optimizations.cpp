// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 RxMiner contributors
// Deterministic JIT state comparison against the frozen pre-port compiler.
// This is a compiler regression probe, not a mining benchmark or a consensus
// dataset: reads use an anonymous, read-only zero dataset to isolate VM code.
#include <jit_compiler_x86.hpp>
#include "program.hpp"
#include "bytecode_machine.hpp"
#include "blake2/blake2.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <sys/mman.h>
#include <xmmintrin.h>

using namespace randomx;

static uint64_t next(uint64_t& state) {
    state ^= state << 13; state ^= state >> 7; state ^= state << 17;
    return state;
}

static void instruction(Program& p, int i, int opcode, int dst, int src, uint32_t imm = 0) {
    p(i).opcode = opcode; p(i).dst = dst; p(i).src = src;
    p(i).mod = 0; p(i).setImm32(imm);
}

static Program makeProgram(unsigned kind, uint64_t seed) {
    Program p{};
    for (unsigned i = 0; i < RANDOMX_PROGRAM_MAX_SIZE; ++i)
        instruction(p, i, ceil_ISWAP_R - 1, 0, 0); // semantic NOP
    if (kind >= 16) {
        auto* bytes = reinterpret_cast<uint8_t*>(&p);
        for (size_t i = 0; i < sizeof(p); ++i) bytes[i] = next(seed);
    } else if (kind < 8) {
        // Distinct modes; cases with intervening uses and backward edges
        // must keep the first setting. v2 must not eliminate conditional sets.
        instruction(p, 0, ceil_FADD_R - 1, 0, 0);
        instruction(p, 1, ceil_CFROUND - 1, 0, 0, kind * 7);
        if (kind == 1) instruction(p, 2, ceil_FADD_R - 1, 0, 1);
        if (kind == 2) instruction(p, 2, ceil_CBRANCH - 1, 0, 0, 0x100);
        if (kind == 3) {
            instruction(p, 1, ceil_IADD_RS - 1, 0, 1);
            instruction(p, 2, ceil_CFROUND - 1, 0, 0);
            instruction(p, 3, ceil_CBRANCH - 1, 0, 0, 0x100);
        }
        if (kind == 4) instruction(p, 2, ceil_FSWAP_R - 1, 0, 0);
        if (kind == 5) instruction(p, 2, ceil_FSCAL_R - 1, 0, 0);
        if (kind == 6) instruction(p, 2, ceil_FDIV_M - 1, 0, 1, 128);
        if (kind != 7) instruction(p, 4, ceil_CFROUND - 1, 0, 1, 13);
        instruction(p, 5, ceil_FMUL_R - 1, 0, 1);
        instruction(p, 6, ceil_FSQRT_R - 1, 1, 0);
    } else {
        const unsigned counts[] = {0, 1, 15, 16, 17, 32, 256, 384};
        for (unsigned i = 0; i < counts[kind - 8]; ++i)
            instruction(p, i, ceil_IMUL_RCP - 1, i % 8, 0, uint32_t(next(seed)) | 3);
        // Exercise zero/power-of-two NOPs without consuming table slots.
        if (kind == 13) {
            p(0).setImm32(0); p(1).setImm32(1); p(2).setImm32(1024);
        }
    }
    return p;
}

int main(int argc, char** argv) {
    bool normalized = false, compact = false;
    for (int i = 1; i < argc; ++i) {
        if (!std::strcmp(argv[i], "--normalized")) normalized = true;
        else if (!std::strcmp(argv[i], "--compact")) compact = true;
        else return 4;
    }
    if (!(randomx_get_flags() & RANDOMX_FLAG_HARD_AES)) {
        std::puts("SKIPPED JIT state regression: hardware AES unavailable");
        return 0;
    }
    void* mapping = mmap(nullptr, DatasetSize, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (mapping == MAP_FAILED) return 2;
    void* allocation = nullptr;
    if (posix_memalign(&allocation, 64, ScratchpadSize)) return 3;
    auto* scratch = static_cast<uint8_t*>(allocation);
    JitCompilerX86 jit(compact);
    const uint32_t savedMxcsr = _mm_getcsr();
    blake2b_state digest;
    blake2b_init(&digest, 32);
    unsigned cases = 0;
    for (unsigned mode = 0; mode < 3; ++mode) {
        auto flags = static_cast<randomx_flags>(RANDOMX_FLAG_FULL_MEM | RANDOMX_FLAG_JIT |
            (mode < 2 ? RANDOMX_FLAG_HARD_AES : 0) | (mode ? RANDOMX_FLAG_V2 : 0));
        jit.setFlags(flags);
        for (unsigned kind = 0; kind < 80; ++kind) {
            uint64_t state = 0xdba56718fced3217ULL + kind * 731;
            auto p = makeProgram(kind, state);
            for (size_t i = 0; i < ScratchpadSize; i += sizeof(state)) {
                next(state); std::memcpy(scratch + i, &state, sizeof(state));
            }
            alignas(64) RegisterFile reg{};
            for (unsigned i = 0; i < RegisterCountFlt; ++i) {
                reg.a[i].lo = 1.25 + i / 16.0;
                reg.a[i].hi = 1.5 + i / 16.0;
            }
            MemoryRegisters mem{};
            mem.memory = static_cast<uint8_t*>(mapping);
            ProgramConfiguration config{};
            config.eMask[0] = config.eMask[1] = 0x3ff0000000000000ULL;
            config.readReg0 = 0; config.readReg1 = 1;
            config.readReg2 = 2; config.readReg3 = 3;
            jit.enableWriting();
            if (normalized) {
                for (unsigned i = 0; i < Program::getSize(flags); ++i) {
                    p(i).src %= 8; p(i).dst %= 8;
                }
            }
            jit.generateProgram(p, config, normalized);
            jit.enableExecution();
            _mm_setcsr(0x9fc0);
            jit.getProgramFunc()(reg, mem, scratch, kind < 16 ? 32 : 3);
            const uint32_t rounding = _mm_getcsr() & 0x6000;
            _mm_setcsr(0x9fc0);
            blake2b_update(&digest, &reg, sizeof(reg));
            blake2b_update(&digest, scratch, ScratchpadSize);
            blake2b_update(&digest, &rounding, sizeof(rounding));
            ++cases;
        }
    }
    uint8_t output[32];
    blake2b_final(&digest, output, sizeof(output));
    std::printf("cases=%u state_blake2b=", cases);
    for (auto byte : output) std::printf("%02x", byte);
    std::puts("");
    _mm_setcsr(savedMxcsr);
    std::free(allocation);
    munmap(mapping, DatasetSize);
    // Frozen pre-port JIT, executable SHA-256:
    // ecae7584cda94bc0865c14950963a07991b7a1da268ac3490c6e20d0c23911c3.
    // This oracle covers default RandomX parameters, final register/scratchpad
    // state and rounding modes, not implementation-specific generated bytes.
    const uint8_t expected[] = {
        0x6a, 0x32, 0x82, 0xb0, 0xd3, 0xe4, 0x33, 0x27,
        0x61, 0xf4, 0xe3, 0x9d, 0x1d, 0x08, 0x35, 0xa9,
        0x84, 0xd8, 0x61, 0xd7, 0xfe, 0x5e, 0x45, 0x63,
        0xc4, 0xe1, 0xd9, 0xfc, 0xf5, 0x5b, 0xe0, 0x75
    };
    if (std::memcmp(output, expected, sizeof(output)) != 0) {
        std::fputs("FAILED JIT state regression\n", stderr);
        return 1;
    }
    std::puts("PASSED JIT state regression (240 programs)");
}
