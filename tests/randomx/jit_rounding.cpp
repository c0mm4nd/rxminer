// SPDX-License-Identifier: BSD-3-Clause
// Verify dead-rounding removal and its floating-point/control-flow boundaries.
#include "jit_compiler_x86.hpp"
#include "bytecode_machine.hpp"
#include <cstdio>
#include <cstdlib>
#include <cstring>

using namespace randomx;
static void set(Program& p, unsigned i, unsigned opcode, unsigned dst = 0,
                unsigned src = 0, uint32_t imm = 0) {
    p(i).opcode = opcode; p(i).dst = dst; p(i).src = src;
    p(i).mod = 0; p(i).setImm32(imm);
}

int main() {
    struct Case { unsigned opcode; bool overwritten; };
    const Case cases[] = {
        {ceil_ISWAP_R - 1, true}, {ceil_IADD_RS - 1, true},
        {ceil_ISTORE - 1, true}, {ceil_FSWAP_R - 1, true},
        {ceil_FSCAL_R - 1, true},
        {ceil_FADD_R - 1, false}, {ceil_FADD_M - 1, false},
        {ceil_FSUB_R - 1, false}, {ceil_FSUB_M - 1, false},
        {ceil_FMUL_R - 1, false}, {ceil_FDIV_M - 1, false},
        {ceil_FSQRT_R - 1, false}, {ceil_CBRANCH - 1, false},
    };
    unsigned count = 0;
    for (bool v2 : {false, true}) for (bool light : {false, true}) {
        for (bool normalized : {false, true}) for (unsigned kind = 0; kind < 16; ++kind) {
            const auto flags = static_cast<randomx_flags>(RANDOMX_FLAG_JIT |
                (v2 ? RANDOMX_FLAG_V2 : 0) | (light ? 0 : RANDOMX_FLAG_FULL_MEM));
            Program original{};
            for (unsigned i = 0; i < RANDOMX_PROGRAM_MAX_SIZE; ++i)
                set(original, i, ceil_ISWAP_R - 1); // no emitted code
            unsigned at = 1;
            bool removable = false;
            if (kind < 13) {
                set(original, 2, cases[kind].opcode, 0, 1, 0x100);
                set(original, 4, ceil_CFROUND - 1, 0, 2, 17);
                removable = !v2 && cases[kind].overwritten;
            } else if (kind == 13) {
                // The mode at the end must reach the next iteration/program.
                at = Program::getSize(flags) - 1;
                set(original, 0, ceil_CFROUND - 1, 0, 2, 17);
            } else if (kind == 14) {
                // A v2-capacity instruction is outside the active v1 program.
                at = RANDOMX_PROGRAM_SIZE_V1 - 1;
                set(original, RANDOMX_PROGRAM_SIZE_V1, ceil_CFROUND - 1, 0, 2, 17);
            } // kind 15: no later rounding set
            set(original, at, ceil_CFROUND - 1, 0, 255, 63);
            if (normalized) {
                for (unsigned i = 0; i < Program::getSize(flags); ++i) {
                    original(i).dst %= RegistersCount;
                    original(i).src %= RegistersCount;
                }
            }
            Program removed = original;
            set(removed, at, ceil_ISWAP_R - 1);
            ProgramConfiguration config{};
            config.eMask[0] = config.eMask[1] = 0x3ff0000000000000ULL;
            config.readReg0 = 0; config.readReg1 = 1;
            config.readReg2 = 2; config.readReg3 = 3;
            // Fresh zero-backed allocations make unused buffer tails comparable.
            JitCompilerX86 actual, canonical;
            actual.setFlags(flags); canonical.setFlags(flags);
            actual.enableWriting(); canonical.enableWriting();
            if (light) {
                actual.generateProgramLight(original, config, 64, normalized);
                canonical.generateProgramLight(removed, config, 64, normalized);
            } else {
                actual.generateProgram(original, config, normalized);
                canonical.generateProgram(removed, config, normalized);
            }
            actual.enableExecution(); canonical.enableExecution();
            const bool identical = std::memcmp(actual.getCode(), canonical.getCode(), actual.getCodeSize()) == 0;
            if (identical != removable) {
                std::fprintf(stderr, "rounding boundary mismatch: v2=%d light=%d normalized=%d kind=%u\n",
                    v2, light, normalized, kind);
                std::abort();
            }
            ++count;
        }
    }
    std::printf("PASSED JIT rounding elimination: %u code/boundary cases\n", count);
}
