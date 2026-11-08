// SPDX-License-Identifier: BSD-3-Clause
// Full-memory code layout, resident-page probe, and maximum-size emission cases.
#include "jit_compiler_x86.hpp"
#include "jit_compiler_x86_static.hpp"
#include "program.hpp"
#include "superscalar_program.hpp"
#include "virtual_memory.h"
#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <stdexcept>
#include <sys/mman.h>
#include <unistd.h>
#include <vector>

using namespace randomx;
static void require(bool value) { if (!value) std::abort(); }

static size_t residentBytes(JitCompilerX86& jit) {
    const long page = sysconf(_SC_PAGESIZE);
    require(page > 0);
    std::vector<unsigned char> pages((jit.getCodeSize() + page - 1) / page);
    require(mincore(jit.getCode(), jit.getCodeSize(), pages.data()) == 0);
    size_t resident = 0;
    for (auto state : pages) if (state & 1) resident += page;
    return resident;
}

int main() {
    rxm_set_jit_huge_pages(0);
    JitCompilerX86 general, compact(true);
    require(compact.getCodeSize() < general.getCodeSize());
    // Residency is an observation, not a correctness gate: the OS may reclaim
    // pages. Sample before inspecting untouched code-buffer bytes.
    std::printf("JIT allocation: general=%zu compact=%zu bytes; initial resident=%zu/%zu bytes\n",
        general.getCodeSize(), compact.getCodeSize(), residentBytes(general), residentBytes(compact));

    auto* epilogue = reinterpret_cast<const uint8_t*>(randomx_program_epilogue);
    const auto epilogueSize = reinterpret_cast<const uint8_t*>(randomx_sshash_load) - epilogue;
    auto* end = compact.getCode() + compact.getCodeSize();
    auto* storedEpilogue = std::search(compact.getCode(), end, epilogue, epilogue + epilogueSize);
    require(storedEpilogue != end);
    uint8_t tail[64];
    std::memcpy(tail, end - sizeof(tail), sizeof(tail));

    ProgramConfiguration config{};
    config.eMask[0] = config.eMask[1] = 0x3ff0000000000000ULL;
    config.readReg0 = 0; config.readReg1 = 1;
    config.readReg2 = 2; config.readReg3 = 3;
    unsigned cases = 0;
    for (bool v2 : {false, true}) for (bool hard : {false, true}) {
        const auto flags = static_cast<randomx_flags>(RANDOMX_FLAG_JIT | RANDOMX_FLAG_FULL_MEM |
            (v2 ? RANDOMX_FLAG_V2 : 0) | (hard ? RANDOMX_FLAG_HARD_AES : 0));
        compact.setFlags(flags);
        for (bool normalized : {false, true}) for (unsigned op = 0; op < 256; ++op) {
            Program p{};
            for (unsigned i = 0; i < Program::getSize(flags); ++i) {
                auto& instr = p(i);
                instr.opcode = op;
                instr.dst = 128 + i % 8;
                instr.src = 64 + (i + 1) % 8;
                instr.mod = i % 256;
                instr.setImm32(0xfffffffd);
                if (normalized) { instr.dst %= 8; instr.src %= 8; }
            }
            compact.enableWriting();
            compact.generateProgram(p, config, normalized);
            compact.enableExecution();
            require(std::memcmp(storedEpilogue, epilogue, epilogueSize) == 0);
            require(std::memcmp(end - sizeof(tail), tail, sizeof(tail)) == 0);
            ++cases;
        }
    }
    // Unsupported uses must fail before touching code, even while it is RX.
    Program p{};
    SuperscalarProgramList programs{};
    std::vector<uint64_t> reciprocals;
    unsigned rejected = 0;
    try { compact.generateProgramLight(p, config, 0); } catch (const std::logic_error&) { ++rejected; }
    try { compact.generateSuperscalarHash(programs, reciprocals); } catch (const std::logic_error&) { ++rejected; }
    try { compact.generateDatasetInitCode(); } catch (const std::logic_error&) { ++rejected; }
    require(rejected == 3);
    std::printf("PASSED compact JIT: %u emission/tail cases; %u unsupported uses rejected\n", cases, rejected);
}
