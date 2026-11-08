// SPDX-License-Identifier: BSD-3-Clause
// Exhaustive integer swap operands followed by branches and temporary-register users.
#include "jit_compiler_x86.hpp"
#include "bytecode_machine.hpp"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <sys/mman.h>
#include <xmmintrin.h>

using namespace randomx;
static void require(bool value) { if (!value) std::abort(); }
static void set(Program& p, unsigned i, unsigned opcode, unsigned dst,
                unsigned src, uint32_t imm = 0) {
    p(i).opcode = opcode; p(i).dst = dst; p(i).src = src;
    p(i).mod = 0; p(i).setImm32(imm);
}
int main() {
    void* dataset = mmap(nullptr, DatasetSize, PROT_READ,
        MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    require(dataset != MAP_FAILED);
    void* allocation = nullptr;
    require(posix_memalign(&allocation, 64, ScratchpadSize) == 0);
    auto* scratch = static_cast<uint8_t*>(allocation);
    const unsigned saved = _mm_getcsr();
    const bool hardware = (randomx_get_flags() & RANDOMX_FLAG_HARD_AES) != 0;
    JitCompilerX86 jit;
    unsigned cases = 0;
    for (bool v2 : {false, true}) for (bool hard : {false, true}) {
        if (hard && !hardware) continue;
        const auto flags = static_cast<randomx_flags>(RANDOMX_FLAG_JIT | RANDOMX_FLAG_FULL_MEM |
            (v2 ? RANDOMX_FLAG_V2 : 0) | (hard ? RANDOMX_FLAG_HARD_AES : 0));
        jit.setFlags(flags);
        for (bool normalized : {false, true}) for (unsigned dst = 0; dst < 8; ++dst) {
            for (unsigned src = 0; src < 8; ++src) {
                Program p{};
                for (unsigned i = 0; i < RANDOMX_PROGRAM_MAX_SIZE; ++i)
                    set(p, i, ceil_ISWAP_R - 1, 0, 0);
                for (unsigned i = 0; i < 8; ++i)
                    set(p, i, ceil_IXOR_R - 1, 128 + i, 64 + i, 0x1234ff00 + (i << 16));
                set(p, 8, ceil_ISWAP_R - 1, 128 + dst, 64 + src);
                // Low bits force a taken backward branch. Equal-register swaps
                // must remain NOPs without changing last-write tracking.
                set(p, 9, ceil_CBRANCH - 1, 128 + dst, 0);
                set(p, 10, ceil_ISMULH_R - 1, 128 + dst, 64 + src);
                set(p, 11, ceil_IROL_R - 1, 128 + dst, 64 + src, 17);
                set(p, 12, ceil_IROR_R - 1, 128 + src, 64 + dst, 19);
                set(p, 13, ceil_IMUL_RCP - 1, 128 + src, 0, 3);
                if (normalized) {
                    for (unsigned i = 0; i < Program::getSize(flags); ++i) {
                        p(i).src %= RegistersCount; p(i).dst %= RegistersCount;
                    }
                }
                NativeRegisterFile reference{};
                InstructionByteCode bytecode[RANDOMX_PROGRAM_MAX_SIZE];
                ProgramConfiguration config{};
                config.eMask[0] = config.eMask[1] = 0x3ff0000000000000ULL;
                config.readReg0 = 0; config.readReg1 = 1;
                config.readReg2 = 2; config.readReg3 = 3;
                BytecodeMachine machine;
                machine.compileProgram(p, bytecode, reference, flags);
                std::memset(scratch, 0, ScratchpadSize);
                machine.executeBytecode(bytecode, scratch, config, flags);
                alignas(64) RegisterFile actual{};
                for (auto& a : actual.a) { a.lo = 1.25; a.hi = 1.5; }
                MemoryRegisters memory{};
                memory.memory = static_cast<uint8_t*>(dataset);
                jit.enableWriting();
                jit.generateProgram(p, config, normalized);
                jit.enableExecution();
                _mm_setcsr(0x9fc0);
                jit.getProgramFunc()(actual, memory, scratch, 1);
                if (std::memcmp(actual.r, reference.r, sizeof(actual.r))) {
                    std::fprintf(stderr, "swap mismatch: v2=%d hard=%d normalized=%d dst=%u src=%u\n",
                        v2, hard, normalized, dst, src);
                    std::abort();
                }
                ++cases;
            }
        }
    }
    _mm_setcsr(saved);
    std::free(allocation);
    require(munmap(dataset, DatasetSize) == 0);
    std::printf("PASSED JIT swaps and branch tracking: %u cases\n", cases);
}
