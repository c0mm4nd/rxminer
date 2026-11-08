// SPDX-License-Identifier: BSD-3-Clause
// Produce explicit little-endian fixtures from the pinned C++ implementation.
// Only used by native_vectors; never linked into the production miner.
#include <cstddef>
#include "blake2_generator.hpp"
#include "superscalar.hpp"
#include "reciprocal.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

static FILE* output;
static void bytes(const void* p, size_t n) { if (std::fwrite(p, 1, n, output) != n) std::abort(); }
static void u32(uint32_t n) { uint8_t p[4]; for (unsigned i = 0; i < 4; ++i) p[i] = uint8_t(n >> (8 * i)); bytes(p, 4); }
static void u64(uint64_t n) { u32(uint32_t(n)); u32(uint32_t(n >> 32)); }
static void programBytes(randomx::SuperscalarProgram& prog) {
    for (unsigned i = 0; i < prog.getSize(); ++i) {
        auto& instruction = prog(i);
        uint8_t head[] = {instruction.opcode, instruction.dst, instruction.src, instruction.mod};
        bytes(head, 4);
        u32(instruction.getImm32());
    }
}
static void emitProgram(randomx::SuperscalarProgram prog, unsigned seed) {
    u32(prog.getSize());
    u32(prog.getAddressRegister());
    programBytes(prog);
    const uint64_t edges[] = {0, 1, UINT64_MAX, UINT64_C(1) << 63, (UINT64_C(1) << 63) - 1, UINT32_MAX, UINT64_C(0xffffffff00000000), UINT64_C(0x123456789abcdef0)};
    uint64_t initial[8], registers[8];
    for (unsigned i = 0; i < 8; ++i) { initial[i] = edges[(i + seed) % 8]; u64(initial[i]); }
    std::memcpy(registers, initial, sizeof(initial));
    randomx::executeSuperscalar(registers, prog);
    for (auto value : registers) u64(value);
    std::vector<uint64_t> reciprocals;
    for (unsigned i = 0; i < prog.getSize(); ++i) {
        auto& instruction = prog(i);
        if (instruction.opcode == unsigned(randomx::SuperscalarInstructionType::IMUL_RCP)) {
            const auto value = randomx_reciprocal(instruction.getImm32());
            instruction.setImm32(uint32_t(reciprocals.size()));
            reciprocals.push_back(value);
        }
    }
    u32(uint32_t(reciprocals.size()));
    for (auto value : reciprocals) u64(value);
    programBytes(prog);
    std::memcpy(registers, initial, sizeof(initial));
    randomx::executeSuperscalar(registers, prog, &reciprocals);
    for (auto value : registers) u64(value);
}

int main(int argc, char** argv) {
    if (argc != 2 || !(output = std::fopen(argv[1], "wb"))) return 1;
    bytes("RXRUST01", 8);
    const unsigned lengths[] = {0, 1, 31, 32, 59, 60, 61, 64, 80, 255};
    const uint32_t nonces[] = {0, 1, UINT32_C(0x80000000), UINT32_MAX};
    u32(10 * 4 * 4);
    for (unsigned length : lengths) for (uint32_t nonce : nonces) for (unsigned pattern = 0; pattern < 4; ++pattern) {
        uint8_t seed[255];
        for (unsigned i = 0; i < length; ++i) seed[i] = uint8_t(i * 37 + length * 13 + pattern);
        u32(length); bytes(seed, length); u32(nonce); u32(512);
        randomx::Blake2Generator generator(seed, length, int(nonce));
        for (unsigned i = 0; i < 512; ++i) {
            const bool word = pattern == 1 || (pattern == 2 && i % 64 >= 61) || (pattern == 3 && i % 7 == 0);
            u32(word ? 4 : 1);
            u32(word ? generator.getUInt32() : generator.getByte());
        }
    }
    constexpr unsigned generated = 256;
    constexpr unsigned crafted = 4 * 14 * 8 * 8;
    u32(generated + crafted);
    for (unsigned seed = 0; seed < generated; ++seed) {
        const uint8_t key[] = {uint8_t(seed), uint8_t(seed >> 8), 0x71, 0xa3};
        randomx::Blake2Generator generator(key, sizeof(key));
        randomx::SuperscalarProgram prog{};
        randomx::generateSuperscalar(prog, generator);
        emitProgram(prog, seed);
    }
    const uint32_t immediates[] = {0, UINT32_C(0x7fffffff), UINT32_C(0x80000000), UINT32_MAX};
    for (unsigned edge = 0; edge < 4; ++edge) for (unsigned op = 0; op < 14; ++op)
        for (unsigned dst = 0; dst < 8; ++dst) for (unsigned src = 0; src < 8; ++src) {
            randomx::SuperscalarProgram prog{};
            prog.setSize(1); prog.setAddressRegister(dst);
            auto& instruction = prog(0);
            instruction.opcode = uint8_t(op); instruction.dst = uint8_t(dst); instruction.src = uint8_t(src);
            instruction.mod = uint8_t(0xf3 | (edge << 2));
            instruction.setImm32(op == 4 ? 1 + edge * 20 : op == 13 ? (immediates[edge] | 3) : immediates[edge]);
            emitProgram(prog, edge);
        }
    if (std::fclose(output) != 0) return 1;
    std::printf("Generated Rust migration oracle: 160 byte streams and %u integer programs\n", generated + crafted);
}
