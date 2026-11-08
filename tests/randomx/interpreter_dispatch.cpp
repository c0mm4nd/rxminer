// SPDX-License-Identifier: BSD-3-Clause
// Compare loop-local dispatch with the retained single-instruction interface.
#include "bytecode_machine.hpp"
#include <algorithm>
#include <cfenv>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <vector>

using namespace randomx;
using Clock = std::chrono::steady_clock;
using Execute = void (*)(InstructionByteCode*, uint8_t*, ProgramConfiguration&, randomx_flags);
static void require(bool value) { if (!value) std::abort(); }
static uint64_t next(uint64_t& state) {
    state ^= state << 13; state ^= state >> 7; state ^= state << 17;
    return state;
}

// This is the previous execution flow: one external dispatch call per opcode.
static void singleStep(InstructionByteCode* bytecode, uint8_t* scratchpad,
                       ProgramConfiguration& config, randomx_flags flags) {
    for (int pc = 0, n = Program::getSize(flags); pc < n; ++pc)
        BytecodeMachine::executeInstruction(bytecode[pc], pc, scratchpad, config, flags);
}

struct Buffer {
    std::unique_ptr<uint8_t[]> allocation;
    uint8_t* data;
    Buffer() : allocation(new uint8_t[ScratchpadSize + 63]),
        data(reinterpret_cast<uint8_t*>((reinterpret_cast<uintptr_t>(allocation.get()) + 63) & ~uintptr_t(63))) {}
};

static Program program(unsigned seed) {
    Program p{};
    uint64_t state = 0x72b6851ed391cc09ULL + seed;
    auto* bytes = reinterpret_cast<uint8_t*>(&p);
    for (size_t i = 0; i < sizeof(p); ++i) bytes[i] = static_cast<uint8_t>(next(state));
    if (seed < 4) {
        // Include NOPs, jumps to pc=-1, tail branches and conditional v2 CFROUND.
        for (unsigned i = 0; i < RANDOMX_PROGRAM_MAX_SIZE; ++i) {
            p(i).opcode = ceil_ISWAP_R - 1;
            p(i).dst = p(i).src = 0;
        }
        const unsigned at = seed == 1 ? RANDOMX_PROGRAM_SIZE_V1 - 1 : seed == 2 ? RANDOMX_PROGRAM_SIZE_V2 - 1 : 0;
        p(at).opcode = ceil_CBRANCH - 1;
        p(at).mod = 0;
        p(at).setImm32(0);
        if (seed == 3) {
            p(1).opcode = ceil_CFROUND - 1;
            p(1).setImm32(0);
            p(2).opcode = ceil_FADD_R - 1;
            p(3).opcode = ceil_CFROUND - 1;
            p(3).src = 1;
            p(3).setImm32(13);
        }
    }
    return p;
}

static NativeRegisterFile registers(unsigned seed) {
    NativeRegisterFile reg{};
    uint64_t state = 0x361cf0917285ab67ULL + seed;
    for (auto& r : reg.r) r = next(state);
    // Force the first crafted branch to jump back to the beginning once.
    if (seed < 4) reg.r[0] = 0xff00;
    for (unsigned i = 0; i < RegisterCountFlt; ++i) {
        reg.f[i] = rx_set_vec_f128(0x3ff4000000000000ULL, 0x3ff8000000000000ULL);
        reg.e[i] = rx_set_vec_f128(0x3ff6000000000000ULL, 0x3ffc000000000000ULL);
        reg.a[i] = rx_set_vec_f128(0x3ff0000000000000ULL, 0x3ff2000000000000ULL);
    }
    return reg;
}

int main(int argc, char** argv) {
    require(argc == 1 || (argc == 2 && std::strcmp(argv[1], "--sample") == 0));
    std::fenv_t saved;
    require(std::fegetenv(&saved) == 0);
    Buffer initial, scratch, expected;
    uint64_t state = 0x217d7a3bcf965b80ULL;
    for (size_t i = 0; i < ScratchpadSize; i += 8) store64(initial.data + i, next(state));
    // Match the VM: exponent masks are read with aligned SIMD loads on x86.
    alignas(16) ProgramConfiguration config{};
    config.eMask[0] = config.eMask[1] = 0x3ff0000000000000ULL;
    unsigned cases = 0;
    for (auto flags : {RANDOMX_FLAG_DEFAULT, RANDOMX_FLAG_V2}) {
        for (unsigned seed = 0; seed < 68; ++seed) {
            auto p = program(seed);
            NativeRegisterFile reg{}, reference{};
            InstructionByteCode bytecode[RANDOMX_PROGRAM_MAX_SIZE];
            BytecodeMachine compiler;
            compiler.compileProgram(p, bytecode, reg, flags);
            for (unsigned rounding = 0; rounding < 4; ++rounding) {
                reg = registers(seed);
                std::memcpy(scratch.data, initial.data, ScratchpadSize);
                rx_reset_float_state();
                rx_set_rounding_mode(rounding);
                singleStep(bytecode, scratch.data, config, flags);
                reference = reg;
                const auto expectedRounding = rx_get_rounding_mode();
                std::memcpy(expected.data, scratch.data, ScratchpadSize);
                reg = registers(seed);
                std::memcpy(scratch.data, initial.data, ScratchpadSize);
                rx_reset_float_state();
                rx_set_rounding_mode(rounding);
                BytecodeMachine::executeBytecode(bytecode, scratch.data, config, flags);
                require(rx_get_rounding_mode() == expectedRounding);
                require(std::memcmp(&reg, &reference, sizeof(reg)) == 0);
                require(std::memcmp(scratch.data, expected.data, ScratchpadSize) == 0);
                ++cases;
            }
        }
    }
    std::printf("PASSED interpreter dispatch: %u register/scratchpad/rounding cases\n", cases);
    if (argc == 2) {
        const auto deadline = Clock::now() + std::chrono::milliseconds(500);
        for (auto flags : {RANDOMX_FLAG_DEFAULT, RANDOMX_FLAG_V2}) {
            std::vector<double> before, after;
            // Average a fixed corpus per pair instead of timing a single opcode mix.
            for (unsigned pair = 0; pair < 9 && Clock::now() < deadline; ++pair) {
                double times[2] = {};
                for (unsigned seed = 4; seed < 20; ++seed) {
                    auto p = program(seed);
                    NativeRegisterFile reg{};
                    InstructionByteCode bytecode[RANDOMX_PROGRAM_MAX_SIZE];
                    BytecodeMachine compiler;
                    compiler.compileProgram(p, bytecode, reg, flags);
                    for (unsigned order = 0; order < 2; ++order) {
                        const auto which = (pair + order) % 2;
                        Execute execute = which ? BytecodeMachine::executeBytecode : singleStep;
                        reg = registers(seed);
                        std::memcpy(scratch.data, initial.data, ScratchpadSize);
                        rx_reset_float_state();
                        const auto start = Clock::now();
                        for (unsigned repeat = 0; repeat < 32; ++repeat)
                            execute(bytecode, scratch.data, config, flags);
                        times[which] += std::chrono::duration<double, std::nano>(Clock::now() - start).count() / (16 * 32);
                    }
                }
                if (pair) { before.push_back(times[0]); after.push_back(times[1]); }
            }
            if (before.empty()) continue;
            const auto median = [](std::vector<double>& times) {
                std::sort(times.begin(), times.end());
                return (times[times.size() / 2] + times[(times.size() - 1) / 2]) / 2;
            };
            rx_reset_float_state();
            std::printf("RXM_DISPATCH_SAMPLE {\"version\":%u,\"pairs\":%u,\"single_step_ns\":%.2f,\"loop_ns\":%.2f}\n",
                flags == RANDOMX_FLAG_V2 ? 2 : 1, unsigned(before.size()), median(before), median(after));
        }
    }
    require(std::fesetenv(&saved) == 0);
}
