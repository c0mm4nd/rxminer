/*
Copyright (c) 2018-2020, tevador <tevador@gmail.com>
Copyright (c) 2019-2021, SChernykh <https://github.com/SChernykh>
Copyright (c) 2019-2021, XMRig <https://github.com/xmrig>, <support@xmrig.com>

All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:
	* Redistributions of source code must retain the above copyright
	  notice, this list of conditions and the following disclaimer.
	* Redistributions in binary form must reproduce the above copyright
	  notice, this list of conditions and the following disclaimer in the
	  documentation and/or other materials provided with the distribution.
	* Neither the name of the copyright holder nor the
	  names of its contributors may be used to endorse or promote products
	  derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*/

#include <stdexcept>
#include <cstring>
#include <climits>
#include "jit_compiler_x86.hpp"
#include "jit_compiler_x86_static.hpp"
#include "superscalar.hpp"
#include "program.hpp"
#include "reciprocal.h"
#include "virtual_memory.h"
#include "soft_aes.h"
#include "cpu.hpp"

namespace randomx {
	/*

	REGISTER ALLOCATION:

	; rax -> temporary
	; rbx -> iteration counter "ic"
	; rcx -> temporary
	; rdx -> temporary
	; rsi -> scratchpad pointer
	; rdi -> dataset pointer
	; rbp -> memory registers "ma" (high 32 bits), "mx" (low 32 bits)
	; rsp -> stack pointer
	; r8  -> "r0"
	; r9  -> "r1"
	; r10 -> "r2"
	; r11 -> "r3"
	; r12 -> "r4"
	; r13 -> "r5"
	; r14 -> "r6"
	; r15 -> "r7"
	; xmm0 -> "f0"
	; xmm1 -> "f1"
	; xmm2 -> "f2"
	; xmm3 -> "f3"
	; xmm4 -> "e0"
	; xmm5 -> "e1"
	; xmm6 -> "e2"
	; xmm7 -> "e3"
	; xmm8 -> "a0"
	; xmm9 -> "a1"
	; xmm10 -> "a2"
	; xmm11 -> "a3"
	; xmm12 -> temporary
	; xmm13 -> E 'and' mask = 0x00ffffffffffffff00ffffffffffffff
	; xmm14 -> E 'or' mask  = 0x3*00000000******3*00000000******
	; xmm15 -> scale mask   = 0x81f000000000000081f0000000000000

	*/

	/*
	STACK LAYOUT (offsets from rsp):
	456 -> return address
	448 -> RegisterFile& registerFile
	432 -> xmm15   store
	416 -> xmm14   store
	400 -> xmm13   store
	384 -> xmm12   store
	368 -> xmm11   store
	352 -> xmm10   store
	336 -> xmm9    store
	320 -> xmm8    store
	304 -> xmm7    store
	288 -> xmm6    store
	280 -> r15     store
	272 -> r14     store
	264 -> r13     store
	256 -> r12     store
	248 -> rdi     store
	240 -> rsi     store
	232 -> rbp     store
	224 -> rbx     store
	208 -> xmm7    spill
	192 -> xmm6    spill
	176 -> xmm5    spill
	160 -> xmm4    spill
	144 -> xmm3    spill
	128 -> xmm2    spill
	112 -> xmm1    spill
	 96 -> xmm0    spill
	 88 -> rbx     spill
	 80 -> r15/rdi spill
	 72 -> r14/rsi spill
	 64 -> r13     spill
	 56 -> r12/rbp spill
	 48 -> r11/rdx spill
	 40 -> r10/rcx spill (full v1: two immutable MXCSR words)
	 32 -> r9      spill (full v1: two immutable MXCSR words)
	 24 -> r8/rax  spill
	 16 -> spAddr1
	  8 -> spAddr0
	  4 -> cached CFROUND selector
	  0 -> mxcsr
	*/

	//Calculate the required code buffer size that is sufficient for the largest possible program:

	// Light v1 CFROUND without BMI2 can use 33 bytes; v2 FDIV_M uses 32.
	constexpr size_t MaxV1ProgramCodeSize = 33 * RANDOMX_PROGRAM_SIZE_V1;
	constexpr size_t MaxV2ProgramCodeSize = 32 * RANDOMX_PROGRAM_SIZE_V2;
	constexpr size_t MaxRandomXProgramCodeSize = MaxV1ProgramCodeSize > MaxV2ProgramCodeSize
		? MaxV1ProgramCodeSize : MaxV2ProgramCodeSize;
	constexpr size_t MaxSuperscalarInstrSize = 14;   //IMUL_RCP requires 14 bytes of x86 code
	constexpr size_t SuperscalarProgramHeader = 128; //overhead per superscalar program
	constexpr size_t CodeAlign = 4096;               //align code size to a multiple of 4 KiB
	constexpr size_t ReserveCodeSize = CodeAlign;    //function prologue/epilogue + reserve

	constexpr size_t RandomXCodeSize = alignSize(ReserveCodeSize + MaxRandomXProgramCodeSize, CodeAlign);
	constexpr size_t SuperscalarSize = alignSize(ReserveCodeSize + (SuperscalarProgramHeader + MaxSuperscalarInstrSize * SuperscalarMaxSize) * RANDOMX_CACHE_ACCESSES, CodeAlign);

	static_assert(RandomXCodeSize < INT32_MAX / 2, "RandomXCodeSize is too large");
	static_assert(SuperscalarSize < INT32_MAX / 2, "SuperscalarSize is too large");

	constexpr uint32_t CodeSize = RandomXCodeSize + SuperscalarSize;

	constexpr int32_t superScalarHashOffset = RandomXCodeSize;

#if defined(_MSC_VER) && (defined(_DEBUG) || defined (RELWITHDEBINFO))
#define ADDR(x) ((((uint8_t*)&x)[0] == 0xE9) ? (((uint8_t*)&x) + *(const int32_t*)(((uint8_t*)&x) + 1) + 5) : ((uint8_t*)&x))
#else
#define ADDR(x) ((uint8_t*)&x)
#endif

	const uint8_t* codePrologue = ADDR(randomx_program_prologue);
	const uint8_t* codeLoopBegin = ADDR(randomx_program_loop_begin);
	const uint8_t* codeLoopLoad = ADDR(randomx_program_loop_load);
	const uint8_t* codeProgamStart = ADDR(randomx_program_start);
	const uint8_t* codeReadDataset = ADDR(randomx_program_read_dataset);
	const uint8_t* codeReadDatasetV2 = ADDR(randomx_program_read_dataset_v2);
	const uint8_t* codeReadDatasetLightSshInit = ADDR(randomx_program_read_dataset_sshash_init);
	const uint8_t* codeReadDatasetLightSshInitV2 = ADDR(randomx_program_read_dataset_sshash_init_v2);
	const uint8_t* codeReadDatasetLightSshFin = ADDR(randomx_program_read_dataset_sshash_fin);
	const uint8_t* codeDatasetInit = ADDR(randomx_dataset_init);
	const uint8_t* codeLoopStore = ADDR(randomx_program_loop_store);
	const uint8_t* codeLoopStoreHardAes = ADDR(randomx_program_loop_store_hard_aes);
	const uint8_t* codeLoopStoreSoftAes = ADDR(randomx_program_loop_store_soft_aes);
	const uint8_t* codeLoopEnd = ADDR(randomx_program_loop_end);
	const uint8_t* codeSoftAes = ADDR(randomx_program_soft_aes);
	const uint8_t* codeSoftAesEnd = ADDR(randomx_program_soft_aes_end);
	const uint8_t* codeEpilogue = ADDR(randomx_program_epilogue);
	const uint8_t* codeProgramEnd = ADDR(randomx_program_end);
	const uint8_t* codeShhLoad = ADDR(randomx_sshash_load);
	const uint8_t* codeShhPrefetch = ADDR(randomx_sshash_prefetch);
	const uint8_t* codeShhEnd = ADDR(randomx_sshash_end);
	const uint8_t* codeShhInit = ADDR(randomx_sshash_init);

	const int32_t prologueSize = codeLoopBegin - codePrologue;
	const int32_t loopLoadSize = codeProgamStart - codeLoopLoad;
	const int32_t readDatasetSize = codeReadDatasetV2 - codeReadDataset;
	const int32_t readDatasetV2Size = codeReadDatasetLightSshInit - codeReadDatasetV2;
	const int32_t readDatasetLightInitSize = codeReadDatasetLightSshInitV2 - codeReadDatasetLightSshInit;
	const int32_t readDatasetLightInitV2Size = codeReadDatasetLightSshFin - codeReadDatasetLightSshInitV2;
	const int32_t readDatasetLightFinSize = codeLoopStore - codeReadDatasetLightSshFin;
	const int32_t loopStoreSize = codeLoopStoreHardAes - codeLoopStore;
	const int32_t loopStoreHardAesSize = codeLoopStoreSoftAes - codeLoopStoreHardAes;
	const int32_t loopStoreSoftAesSize = codeLoopEnd - codeLoopStoreSoftAes;
	const int32_t softAesSize = codeSoftAesEnd - codeSoftAes;

	const int32_t datasetInitSize = codeEpilogue - codeDatasetInit;
	const int32_t epilogueSize = codeShhLoad - codeEpilogue;
	const int32_t codeSshLoadSize = codeShhPrefetch - codeShhLoad;
	const int32_t codeSshPrefetchSize = codeShhEnd - codeShhPrefetch;
	const int32_t codeSshInitSize = codeProgramEnd - codeShhInit;

	const int32_t epilogueOffset = CodeSize - epilogueSize;
	// The program reserve leaves room for four immutable rounding controls.
	constexpr int32_t roundingTableOffset = RandomXCodeSize - 64;
	constexpr uint32_t reciprocalCapacity = 16;
	constexpr int32_t reciprocalTableOffset = roundingTableOffset - reciprocalCapacity * sizeof(uint64_t);

	static const uint8_t REX_ADD_RR[] = { 0x4d, 0x03 };
	static const uint8_t REX_ADD_RM[] = { 0x4c, 0x03 };
	static const uint8_t REX_SUB_RR[] = { 0x4d, 0x2b };
	static const uint8_t REX_SUB_RM[] = { 0x4c, 0x2b };
	static const uint8_t REX_MOV_RR[] = { 0x41, 0x8b };
	static const uint8_t REX_MOV_RR64[] = { 0x49, 0x8b };
	static const uint8_t REX_MOV_R64R[] = { 0x4c, 0x8b };
	static const uint8_t REX_IMUL_RR[] = { 0x4d, 0x0f, 0xaf };
	static const uint8_t REX_IMUL_RRI[] = { 0x4d, 0x69 };
	static const uint8_t REX_IMUL_RM[] = { 0x4c, 0x0f, 0xaf };
	static const uint8_t REX_MUL_R[] = { 0x49, 0xf7 };
	static const uint8_t REX_MUL_M[] = { 0x48, 0xf7 };
	static const uint8_t REX_81[] = { 0x49, 0x81 };
	static const uint8_t AND_EAX_I = 0x25;
	static const uint8_t MOV_EAX_I = 0xb8;
	static const uint8_t MOV_RAX_I[] = { 0x48, 0xb8 };
	static const uint8_t MOV_RCX_I[] = { 0x48, 0xb9 };
	static const uint8_t REX_LEA[] = { 0x4f, 0x8d };
	static const uint8_t REX_MUL_MEM[] = { 0x48, 0xf7, 0x24, 0x0e };
	static const uint8_t REX_IMUL_MEM[] = { 0x48, 0xf7, 0x2c, 0x0e };
	static const uint8_t REX_SHR_RAX[] = { 0x48, 0xc1, 0xe8 };
	static const uint8_t RAX_ADD_SBB_1[] = { 0x48, 0x83, 0xC0, 0x01, 0x48, 0x83, 0xD8, 0x00 };
	static const uint8_t MUL_RCX[] = { 0x48, 0xf7, 0xe1 };
	static const uint8_t REX_SHR_RDX[] = { 0x48, 0xc1, 0xea };
	static const uint8_t REX_SH[] = { 0x49, 0xc1 };
	static const uint8_t MOV_RCX_RAX_SAR_RCX_63[] = { 0x48, 0x89, 0xc1, 0x48, 0xc1, 0xf9, 0x3f };
	static const uint8_t AND_ECX_I[] = { 0x81, 0xe1 };
	static const uint8_t ADD_RAX_RCX[] = { 0x48, 0x01, 0xC8 };
	static const uint8_t SAR_RAX_I8[] = { 0x48, 0xC1, 0xF8 };
	static const uint8_t NEG_RAX[] = { 0x48, 0xF7, 0xD8 };
	static const uint8_t ADD_R_RAX[] = { 0x4C, 0x03 };
	static const uint8_t XOR_EAX_EAX[] = { 0x33, 0xC0 };
	static const uint8_t ADD_RDX_R[] = { 0x4c, 0x01 };
	static const uint8_t SUB_RDX_R[] = { 0x4c, 0x29 };
	static const uint8_t SAR_RDX_I8[] = { 0x48, 0xC1, 0xFA };
	static const uint8_t TEST_RDX_RDX[] = { 0x48, 0x85, 0xD2 };
	static const uint8_t SETS_AL_ADD_RDX_RAX[] = { 0x0F, 0x98, 0xC0, 0x48, 0x03, 0xD0 };
	static const uint8_t REX_NEG[] = { 0x49, 0xF7 };
	static const uint8_t REX_XOR_RR[] = { 0x4D, 0x33 };
	static const uint8_t REX_XOR_RI[] = { 0x49, 0x81 };
	static const uint8_t REX_XOR_RM[] = { 0x4c, 0x33 };
	static const uint8_t REX_ROT_CL[] = { 0x49, 0xd3 };
	static const uint8_t REX_ROT_I8[] = { 0x49, 0xc1 };
	static const uint8_t SHUFPD[] = { 0x66, 0x0f, 0xc6 };
	static const uint8_t REX_ADDPD[] = { 0x66, 0x41, 0x0f, 0x58 };
	static const uint8_t REX_CVTDQ2PD_XMM12[] = { 0xf3, 0x44, 0x0f, 0xe6, 0x24, 0x06 };
	static const uint8_t REX_SUBPD[] = { 0x66, 0x41, 0x0f, 0x5c };
	static const uint8_t REX_XORPS[] = { 0x41, 0x0f, 0x57 };
	static const uint8_t REX_MULPD[] = { 0x66, 0x41, 0x0f, 0x59 };
	static const uint8_t REX_MAXPD[] = { 0x66, 0x41, 0x0f, 0x5f };
	static const uint8_t REX_DIVPD[] = { 0x66, 0x41, 0x0f, 0x5e };
	static const uint8_t SQRTPD[] = { 0x66, 0x0f, 0x51 };
	static const uint8_t AND_OR_MOV_LDMXCSR[] = { 0x25, 0x00, 0x60, 0x00, 0x00, 0x0d, 0xc0, 0x9f, 0x00, 0x00, 0x89, 0x04, 0x24, 0x0f, 0xae, 0x14, 0x24 };
	static const uint8_t ROL_RAX[] = { 0x48, 0xc1, 0xc0 };
	static const uint8_t XOR_ECX_ECX[] = { 0x33, 0xC9 };
	static const uint8_t REX_CMP_R32I[] = { 0x41, 0x81 };
	static const uint8_t REX_CMP_M32I[] = { 0x81, 0x3c, 0x06 };
	static const uint8_t MOVAPD[] = { 0x66, 0x0f, 0x29 };
	static const uint8_t REX_MOV_MR[] = { 0x4c, 0x89 };
	static const uint8_t REX_XOR_EAX[] = { 0x41, 0x33 };
	static const uint8_t SUB_EBX[] = { 0x83, 0xEB, 0x01 };
	static const uint8_t JNZ[] = { 0x0f, 0x85 };
	static const uint8_t JNZ_SHORT = 0x75;
	static const uint8_t JMP = 0xe9;
	static const uint8_t REX_XOR_RAX_R64[] = { 0x49, 0x33 };
	static const uint8_t REX_XCHG[] = { 0x4d, 0x87 };
	static const uint8_t REX_ANDPS_XMM12[] = { 0x45, 0x0F, 0x54, 0xE5, 0x45, 0x0F, 0x56, 0xE6 };
	static const uint8_t REX_PADD[] = { 0x66, 0x44, 0x0f };
	static const uint8_t PADD_OPCODES[] = { 0xfc, 0xfd, 0xfe, 0xd4 };
	static const uint8_t CALL = 0xe8;
	static const uint8_t REX_ADD_I[] = { 0x49, 0x81 };
	static const uint8_t REX_TEST[] = { 0x49, 0xF7 };
	static const uint8_t JZ[] = { 0x0f, 0x84 };
	static const uint8_t RET = 0xc3;
	static const uint8_t LEA_32[] = { 0x41, 0x8d };
	static const uint8_t MOVNTI[] = { 0x4c, 0x0f, 0xc3 };
	static const uint8_t ADD_EBX_I[] = { 0x81, 0xc3 };
	static const uint8_t TEST_EAX_60SL13[] = { 0xa9, 0x00, 0x80, 0x07, 0x00 };

	static const uint8_t NOP1[] = { 0x90 };
	static const uint8_t NOP2[] = { 0x66, 0x90 };
	static const uint8_t NOP3[] = { 0x66, 0x66, 0x90 };
	static const uint8_t NOP4[] = { 0x0F, 0x1F, 0x40, 0x00 };
	static const uint8_t NOP5[] = { 0x0F, 0x1F, 0x44, 0x00, 0x00 };
	static const uint8_t NOP6[] = { 0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00 };
	static const uint8_t NOP7[] = { 0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00 };
	static const uint8_t NOP8[] = { 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00 };

	static const uint8_t* NOPX[] = { NOP1, NOP2, NOP3, NOP4, NOP5, NOP6, NOP7, NOP8 };

	size_t JitCompilerX86::getCodeSize() {
		return CodeSize;
	}

	void JitCompilerX86::alignCode(int align) {
		int rem = codePos % align;
		while (rem != 0) {
			int nopSize = align - rem;
			if (nopSize > 8) nopSize = 8;
			emit(NOPX[nopSize - 1], nopSize);
			rem = codePos % align;
		}
	}

	JitCompilerX86::JitCompilerX86() {
#if defined(__GNUC__) || defined(__clang__)
		hasBmi2 = cpu.hasBmi2();
#if defined(__APPLE__)
		tuneRounding = false;
#else
		tuneRounding = __builtin_cpu_is("znver2");
#endif
#endif
		prefetchMode = rxm_get_prefetch_mode();
		code = (uint8_t*)allocJitMemory(CodeSize, &mappedSize);
		if (code == nullptr)
			throw std::runtime_error("allocMemoryPages");
		memcpy(code, codePrologue, prologueSize);
		memcpy(code + epilogueOffset, codeEpilogue, epilogueSize);
		const uint32_t roundingControls[4] = {0x9fc0, 0xbfc0, 0xdfc0, 0xffc0};
		memcpy(code + roundingTableOffset, roundingControls, sizeof(roundingControls));
	}

	JitCompilerX86::~JitCompilerX86() {
		freePagedMemory(code, mappedSize);
	}

	void JitCompilerX86::enableAll() {
		if (setPagesRWX(code, mappedSize))
			throw std::runtime_error("setPagesRWX");
	}

	void JitCompilerX86::enableWriting() {
		if (setPagesRW(code, mappedSize))
			throw std::runtime_error("setPagesRW");
	}

	void JitCompilerX86::enableExecution() {
		if (setPagesRX(code, mappedSize))
			throw std::runtime_error("setPagesRX");
	}

	void JitCompilerX86::generateProgram(Program& prog, ProgramConfiguration& pcfg, bool registersNormalized) {
		generateProgramPrologue(prog, pcfg, registersNormalized);
		if (vmFlags & RANDOMX_FLAG_V2) {
			memcpy(code + codePos, codeReadDatasetV2, readDatasetV2Size);
			codePos += readDatasetV2Size;
		}
		else {
			memcpy(code + codePos, codeReadDataset, readDatasetSize);
			codePos += readDatasetSize;
		}
		generateProgramEpilogue(prog, pcfg);
	}

	void JitCompilerX86::generateProgramLight(Program& prog, ProgramConfiguration& pcfg, uint32_t datasetOffset, bool registersNormalized) {
		generateProgramPrologue(prog, pcfg, registersNormalized);
		if (vmFlags & RANDOMX_FLAG_V2) {
			emit(codeReadDatasetLightSshInitV2, readDatasetLightInitV2Size);
		}
		else {
			emit(codeReadDatasetLightSshInit, readDatasetLightInitSize);
		}
		emit(ADD_EBX_I);
		emit32(datasetOffset / CacheLineSize);
		emitByte(CALL);
		emit32(superScalarHashOffset - (codePos + 4));
		emit(codeReadDatasetLightSshFin, readDatasetLightFinSize);
		generateProgramEpilogue(prog, pcfg);
	}

	void JitCompilerX86::generateSuperscalarHash(SuperscalarProgramList &programs, std::vector<uint64_t> &reciprocalCache) {
		memcpy(code + superScalarHashOffset, codeShhInit, codeSshInitSize);
		codePos = superScalarHashOffset + codeSshInitSize;
		for (unsigned j = 0; j < programs.size(); ++j) {
			SuperscalarProgram& prog = programs[j];
			for (unsigned i = 0; i < prog.getSize(); ++i) {
				Instruction& instr = prog(i);
				generateSuperscalarCode(instr, reciprocalCache);
			}
			emit(codeShhLoad, codeSshLoadSize);
			if (j < programs.size() - 1) {
				emit(REX_MOV_RR64);
				emitByte(0xd8 + prog.getAddressRegister());
				emit(codeShhPrefetch, codeSshPrefetchSize);
#ifdef RANDOMX_ALIGN
				alignCode(16);
#endif
			}
		}
		emitByte(RET);
	}

	void JitCompilerX86::generateDatasetInitCode() {
		memcpy(code, codeDatasetInit, datasetInitSize);
	}

	void JitCompilerX86::generateProgramPrologue(Program& prog, ProgramConfiguration& pcfg, bool registersNormalized) {
		reciprocalCount = 0;
		for (unsigned i = 0; i < RegistersCount; ++i) {
			registerUsage[i] = -1;
		}

		codePos = prologueSize;
		memcpy(code + codePos - 48, &pcfg.eMask, sizeof(pcfg.eMask));
		stackReciprocals = false;
#if !defined(WINABI)
		// Match XMRig's stack-constant approach for leaf System V VMs. Light
		// mode and v2 software AES make calls and cannot keep data in the
		// 128-byte red zone. They use the RIP-relative table instead.
		stackReciprocals = (vmFlags & RANDOMX_FLAG_FULL_MEM) &&
			(!(vmFlags & RANDOMX_FLAG_V2) || (vmFlags & RANDOMX_FLAG_HARD_AES));
#endif
		if (stackReciprocals) {
			for (uint32_t slot = 0; slot < reciprocalCapacity; ++slot) {
				const uint8_t load[] = {0x48, 0x8b, 0x0d};
				emit(load); // mov rcx, [rip+constant]
				emit32(reciprocalTableOffset + slot * sizeof(uint64_t) - (codePos + 4));
				const uint8_t store[] = {0x48, 0x89, 0x4c, 0x24};
				emit(store); // mov [rsp-8*(slot+1)], rcx
				emitByte(static_cast<uint8_t>(-8 * (slot + 1)));
			}
		}
		loopStart = codePos;
		memcpy(code + codePos, codeLoopLoad, loopLoadSize);
		codePos += loopLoadSize;
		if (registersNormalized) {
			for (unsigned i = 0, n = prog.getSize(vmFlags); i < n; ++i)
				generateCode(prog(i), i);
		} else {
			for (unsigned i = 0; i < prog.getSize(vmFlags); ++i) {
				Instruction& instr = prog(i);
				instr.src %= RegistersCount;
				instr.dst %= RegistersCount;
				generateCode(instr, i);
			}
		}
		emit(REX_MOV_RR);
		emitByte(0xc0 + pcfg.readReg2);
		emit(REX_XOR_EAX);
		emitByte(0xc0 + pcfg.readReg3);
	}

	void JitCompilerX86::generateProgramEpilogue(Program& prog, ProgramConfiguration& pcfg) {
		emit(REX_MOV_RR64);
		emitByte(0xc0 + pcfg.readReg0);
		emit(REX_XOR_RAX_R64);
		emitByte(0xc0 + pcfg.readReg1);
		if (hasBmi2) {
            const uint8_t rotate[] = {0xc4, 0xe3, 0xfb, 0xf0, 0xd0, 0x20};
            emit(rotate); // rorx rdx, rax, 32
        } else {
            const uint8_t split[] = {0x48, 0x89, 0xc2, 0x48, 0xc1, 0xea, 0x20};
            emit(split); // mov rdx, rax; shr rdx, 32
        }
        emitByte(AND_EAX_I);
        emit32(RANDOMX_SCRATCHPAD_L3 - CacheLineSize);
        const auto prefetch = [this](uint8_t sib) {
            if (prefetchMode == 0) return;
            if (prefetchMode == 3) {
                const uint8_t load[] = {0x48, 0x8b, 0x0c};
                emit(load); // mov rcx, [rsi + index]
            } else {
                emitByte(0x0f); emitByte(0x18);
                emitByte(prefetchMode == 2 ? 0x04 : 0x0c); // NTA or T0
            }
            emitByte(sib);
        };
        prefetch(0x06);
        const uint8_t mask[] = {0x81, 0xe2};
        emit(mask);
        emit32(RANDOMX_SCRATCHPAD_L3 - CacheLineSize);
        prefetch(0x16);
		if (vmFlags & RANDOMX_FLAG_V2) {
			if (vmFlags & RANDOMX_FLAG_HARD_AES) {
				memcpy(code + codePos, codeLoopStoreHardAes, loopStoreHardAesSize);
				codePos += loopStoreHardAesSize;
			}
			else {
				memcpy(code + codePos, codeLoopStoreSoftAes, loopStoreSoftAesSize);
				codePos += loopStoreSoftAesSize;
			}
		}
		else {
			memcpy(code + codePos, codeLoopStore, loopStoreSize);
			codePos += loopStoreSize;
		}
		emit(SUB_EBX);
		emit(JNZ);
		emit32(loopStart - codePos - 4);
		emitByte(JMP);
		emit32(epilogueOffset - codePos - 4);
		if ((vmFlags & RANDOMX_FLAG_V2) && !(vmFlags & RANDOMX_FLAG_HARD_AES)) {
			memcpy(code + codePos, codeSoftAes, softAesSize);
			codePos += softAesSize;
			emit64((uint64_t)randomx_aes_lut_enc);
			emit64((uint64_t)randomx_aes_lut_dec);
		}
	}

	void JitCompilerX86::generateCode(Instruction& instr, int i) {
		instructionOffsets[i] = codePos;
		auto generator = engine[instr.opcode];
		generator(this, instr, i);
	}

	void JitCompilerX86::generateSuperscalarCode(Instruction& instr, std::vector<uint64_t> &reciprocalCache) {
		switch ((SuperscalarInstructionType)instr.opcode)
		{
		case randomx::SuperscalarInstructionType::ISUB_R:
			emit(REX_SUB_RR);
			emitByte(0xc0 + 8 * instr.dst + instr.src);
			break;
		case randomx::SuperscalarInstructionType::IXOR_R:
			emit(REX_XOR_RR);
			emitByte(0xc0 + 8 * instr.dst + instr.src);
			break;
		case randomx::SuperscalarInstructionType::IADD_RS:
			emit(REX_LEA);
			emitByte(0x04 + 8 * instr.dst);
			genSIB(instr.getModShift(), instr.src, instr.dst);
			break;
		case randomx::SuperscalarInstructionType::IMUL_R:
			emit(REX_IMUL_RR);
			emitByte(0xc0 + 8 * instr.dst + instr.src);
			break;
		case randomx::SuperscalarInstructionType::IROR_C:
			emit(REX_ROT_I8);
			emitByte(0xc8 + instr.dst);
			emitByte(instr.getImm32() & 63);
			break;
		case randomx::SuperscalarInstructionType::IADD_C7:
			emit(REX_81);
			emitByte(0xc0 + instr.dst);
			emit32(instr.getImm32());
			break;
		case randomx::SuperscalarInstructionType::IXOR_C7:
			emit(REX_XOR_RI);
			emitByte(0xf0 + instr.dst);
			emit32(instr.getImm32());
			break;
		case randomx::SuperscalarInstructionType::IADD_C8:
			emit(REX_81);
			emitByte(0xc0 + instr.dst);
			emit32(instr.getImm32());
#ifdef RANDOMX_ALIGN
			emit(NOP1);
#endif
			break;
		case randomx::SuperscalarInstructionType::IXOR_C8:
			emit(REX_XOR_RI);
			emitByte(0xf0 + instr.dst);
			emit32(instr.getImm32());
#ifdef RANDOMX_ALIGN
			emit(NOP1);
#endif
			break;
		case randomx::SuperscalarInstructionType::IADD_C9:
			emit(REX_81);
			emitByte(0xc0 + instr.dst);
			emit32(instr.getImm32());
#ifdef RANDOMX_ALIGN
			emit(NOP2);
#endif
			break;
		case randomx::SuperscalarInstructionType::IXOR_C9:
			emit(REX_XOR_RI);
			emitByte(0xf0 + instr.dst);
			emit32(instr.getImm32());
#ifdef RANDOMX_ALIGN
			emit(NOP2);
#endif
			break;
		case randomx::SuperscalarInstructionType::IMULH_R:
			emit(REX_MOV_RR64);
			emitByte(0xc0 + instr.dst);
			emit(REX_MUL_R);
			emitByte(0xe0 + instr.src);
			emit(REX_MOV_R64R);
			emitByte(0xc2 + 8 * instr.dst);
			break;
		case randomx::SuperscalarInstructionType::ISMULH_R:
			emit(REX_MOV_RR64);
			emitByte(0xc0 + instr.dst);
			emit(REX_MUL_R);
			emitByte(0xe8 + instr.src);
			emit(REX_MOV_R64R);
			emitByte(0xc2 + 8 * instr.dst);
			break;
		case randomx::SuperscalarInstructionType::IMUL_RCP:
			emit(MOV_RAX_I);
			emit64(reciprocalCache[instr.getImm32()]);
			emit(REX_IMUL_RM);
			emitByte(0xc0 + 8 * instr.dst);
			break;
		default:
			UNREACHABLE;
		}
	}

	inline void JitCompilerX86::InstructionEmitter::genAddressReg(Instruction& instr, bool rax) {
		emit(LEA_32);
		emitByte(0x80 + instr.src + (rax ? 0 : 8));
		if (instr.src == RegisterNeedsSib) {
			emitByte(0x24);
		}
		emit32(instr.getImm32());
		if (rax)
			emitByte(AND_EAX_I);
		else
			emit(AND_ECX_I);
		emit32(instr.getModMem() ? ScratchpadL1Mask : ScratchpadL2Mask);
	}

	inline void JitCompilerX86::InstructionEmitter::genAddressRegDst(Instruction& instr) {
		emit(LEA_32);
		emitByte(0x80 + instr.dst);
		if (instr.dst == RegisterNeedsSib) {
			emitByte(0x24);
		}
		emit32(instr.getImm32());
		emitByte(AND_EAX_I);
		if (instr.getModCond() < StoreL3Condition) {
			emit32(instr.getModMem() ? ScratchpadL1Mask : ScratchpadL2Mask);
		}
		else {
			emit32(ScratchpadL3Mask);
		}
	}

	inline void JitCompilerX86::InstructionEmitter::genAddressImm(Instruction& instr) {
		emit32(instr.getImm32() & ScratchpadL3Mask);
	}

	void JitCompilerX86::h_IADD_RS(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		output.emit(REX_LEA);
		if (instr.dst == RegisterNeedsDisplacement)
			output.emitByte(0xac);
		else
			output.emitByte(0x04 + 8 * instr.dst);
		output.genSIB(instr.getModShift(), instr.src, instr.dst);
		if (instr.dst == RegisterNeedsDisplacement)
			output.emit32(instr.getImm32());
	}

	void JitCompilerX86::h_IADD_M(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (instr.src != instr.dst) {
			output.genAddressReg(instr, true);
			output.emit(REX_ADD_RM);
			output.emitByte(0x04 + 8 * instr.dst);
			output.emitByte(0x06);
		}
		else {
			output.emit(REX_ADD_RM);
			output.emitByte(0x86 + 8 * instr.dst);
			output.genAddressImm(instr);
		}
	}

	void JitCompilerX86::genSIB(int scale, int index, int base) {
		emitByte((scale << 6) | (index << 3) | base);
	}

	void JitCompilerX86::h_ISUB_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (instr.src != instr.dst) {
			output.emit(REX_SUB_RR);
			output.emitByte(0xc0 + 8 * instr.dst + instr.src);
		}
		else {
			output.emit(REX_81);
			output.emitByte(0xe8 + instr.dst);
			output.emit32(instr.getImm32());
		}
	}

	void JitCompilerX86::h_ISUB_M(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (instr.src != instr.dst) {
			output.genAddressReg(instr, true);
			output.emit(REX_SUB_RM);
			output.emitByte(0x04 + 8 * instr.dst);
			output.emitByte(0x06);
		}
		else {
			output.emit(REX_SUB_RM);
			output.emitByte(0x86 + 8 * instr.dst);
			output.genAddressImm(instr);
		}
	}

	void JitCompilerX86::h_IMUL_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (instr.src != instr.dst) {
			output.emit(REX_IMUL_RR);
			output.emitByte(0xc0 + 8 * instr.dst + instr.src);
		}
		else {
			output.emit(REX_IMUL_RRI);
			output.emitByte(0xc0 + 9 * instr.dst);
			output.emit32(instr.getImm32());
		}
	}

	void JitCompilerX86::h_IMUL_M(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (instr.src != instr.dst) {
			output.genAddressReg(instr, true);
			output.emit(REX_IMUL_RM);
			output.emitByte(0x04 + 8 * instr.dst);
			output.emitByte(0x06);
		}
		else {
			output.emit(REX_IMUL_RM);
			output.emitByte(0x86 + 8 * instr.dst);
			output.genAddressImm(instr);
		}
	}

	void JitCompilerX86::h_IMULH_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (hasBmi2) {
			// RDX supplies the multiplicand; MULX writes the high half directly
			// to the VM destination and discards the low half in RAX.
			output.emit(REX_MOV_RR64);
			output.emitByte(0xd0 + instr.dst); // mov rdx, r{dst}
			const uint8_t multiply[] = {0xc4, 0x42, 0xfb, 0xf6};
			output.emit(multiply);
			output.emitByte(0xc0 + 8 * instr.dst + instr.src);
			return;
		}
		output.emit(REX_MOV_RR64);
		output.emitByte(0xc0 + instr.dst);
		output.emit(REX_MUL_R);
		output.emitByte(0xe0 + instr.src);
		output.emit(REX_MOV_R64R);
		output.emitByte(0xc2 + 8 * instr.dst);
	}

	void JitCompilerX86::h_IMULH_M(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (hasBmi2) {
			if (instr.src != instr.dst) output.genAddressReg(instr, false);
			output.emit(REX_MOV_RR64);
			output.emitByte(0xd0 + instr.dst);
			const uint8_t multiply[] = {0xc4, 0x62, 0xfb, 0xf6};
			output.emit(multiply);
			if (instr.src != instr.dst) {
				output.emitByte(0x04 + 8 * instr.dst);
				output.emitByte(0x0e); // [rsi+rcx]
			} else {
				output.emitByte(0x86 + 8 * instr.dst);
				output.genAddressImm(instr);
			}
			return;
		}
		if (instr.src != instr.dst) {
			output.genAddressReg(instr, false);
			output.emit(REX_MOV_RR64);
			output.emitByte(0xc0 + instr.dst);
			output.emit(REX_MUL_MEM);
		}
		else {
			output.emit(REX_MOV_RR64);
			output.emitByte(0xc0 + instr.dst);
			output.emit(REX_MUL_M);
			output.emitByte(0xa6);
			output.genAddressImm(instr);
		}
		output.emit(REX_MOV_R64R);
		output.emitByte(0xc2 + 8 * instr.dst);
	}

	void JitCompilerX86::h_ISMULH_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		output.emit(REX_MOV_RR64);
		output.emitByte(0xc0 + instr.dst);
		output.emit(REX_MUL_R);
		output.emitByte(0xe8 + instr.src);
		output.emit(REX_MOV_R64R);
		output.emitByte(0xc2 + 8 * instr.dst);
	}

	void JitCompilerX86::h_ISMULH_M(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (instr.src != instr.dst) {
			output.genAddressReg(instr, false);
			output.emit(REX_MOV_RR64);
			output.emitByte(0xc0 + instr.dst);
			output.emit(REX_IMUL_MEM);
		}
		else {
			output.emit(REX_MOV_RR64);
			output.emitByte(0xc0 + instr.dst);
			output.emit(REX_MUL_M);
			output.emitByte(0xae);
			output.genAddressImm(instr);
		}
		output.emit(REX_MOV_R64R);
		output.emitByte(0xc2 + 8 * instr.dst);
	}

	void JitCompilerX86::h_IMUL_RCP(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		uint64_t divisor = instr.getImm32();
		if (!isZeroOrPowerOf2(divisor)) {
			registerUsage[instr.dst] = i;
			// XMRig hoists up to 16 reciprocal constants out of the VM loop.
			// The private table is copied into the red zone once per program
			// for leaf System V VMs; other modes read it with RIP addressing.
			// All constants are written before execution, including W^X mode.
			if (reciprocalCount < reciprocalCapacity) {
				const uint64_t reciprocal = randomx_reciprocal_fast(divisor);
				const uint32_t slot = reciprocalCount++;
				const int32_t offset = reciprocalTableOffset + slot * sizeof(reciprocal);
				memcpy(code + offset, &reciprocal, sizeof(reciprocal));
				output.emit(REX_IMUL_RM);
				if (stackReciprocals) {
					output.emitByte(0x44 + 8 * instr.dst);
					output.emitByte(0x24);
					output.emitByte(static_cast<uint8_t>(-8 * (slot + 1)));
				} else {
					output.emitByte(0x05 + 8 * instr.dst); // imul r8-r15, [rip+disp32]
					output.emit32(offset - (output.pos + 4));
				}
				return;
			}
			output.emit(MOV_RAX_I);
			output.emit64(randomx_reciprocal_fast(divisor));
			output.emit(REX_IMUL_RM);
			output.emitByte(0xc0 + 8 * instr.dst);
		}
	}

	void JitCompilerX86::h_INEG_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		output.emit(REX_NEG);
		output.emitByte(0xd8 + instr.dst);
	}

	void JitCompilerX86::h_IXOR_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (instr.src != instr.dst) {
			output.emit(REX_XOR_RR);
			output.emitByte(0xc0 + 8 * instr.dst + instr.src);
		}
		else {
			output.emit(REX_XOR_RI);
			output.emitByte(0xf0 + instr.dst);
			output.emit32(instr.getImm32());
		}
	}

	void JitCompilerX86::h_IXOR_M(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (instr.src != instr.dst) {
			output.genAddressReg(instr, true);
			output.emit(REX_XOR_RM);
			output.emitByte(0x04 + 8 * instr.dst);
			output.emitByte(0x06);
		}
		else {
			output.emit(REX_XOR_RM);
			output.emitByte(0x86 + 8 * instr.dst);
			output.genAddressImm(instr);
		}
	}

	void JitCompilerX86::h_IROR_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (instr.src != instr.dst) {
			output.emit(REX_MOV_RR);
			output.emitByte(0xc8 + instr.src);
			output.emit(REX_ROT_CL);
			output.emitByte(0xc8 + instr.dst);
		}
		else {
			output.emit(REX_ROT_I8);
			output.emitByte(0xc8 + instr.dst);
			output.emitByte(instr.getImm32() & 63);
		}
	}

	void JitCompilerX86::h_IROL_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		registerUsage[instr.dst] = i;
		if (instr.src != instr.dst) {
			output.emit(REX_MOV_RR);
			output.emitByte(0xc8 + instr.src);
			output.emit(REX_ROT_CL);
			output.emitByte(0xc0 + instr.dst);
		}
		else {
			output.emit(REX_ROT_I8);
			output.emitByte(0xc0 + instr.dst);
			output.emitByte(instr.getImm32() & 63);
		}
	}

	void JitCompilerX86::h_ISWAP_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		if (instr.src != instr.dst) {
			registerUsage[instr.dst] = i;
			registerUsage[instr.src] = i;
			output.emit(REX_XCHG);
			output.emitByte(0xc0 + instr.src + 8 * instr.dst);
		}
	}

	void JitCompilerX86::h_FSWAP_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		output.emit(SHUFPD);
		output.emitByte(0xc0 + 9 * instr.dst);
		output.emitByte(1);
	}

	void JitCompilerX86::h_FADD_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		instr.dst %= RegisterCountFlt;
		instr.src %= RegisterCountFlt;
		output.emit(REX_ADDPD);
		output.emitByte(0xc0 + instr.src + 8 * instr.dst);
	}

	void JitCompilerX86::h_FADD_M(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		instr.dst %= RegisterCountFlt;
		output.genAddressReg(instr, true);
		output.emit(REX_CVTDQ2PD_XMM12);
		output.emit(REX_ADDPD);
		output.emitByte(0xc4 + 8 * instr.dst);
	}

	void JitCompilerX86::h_FSUB_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		instr.dst %= RegisterCountFlt;
		instr.src %= RegisterCountFlt;
		output.emit(REX_SUBPD);
		output.emitByte(0xc0 + instr.src + 8 * instr.dst);
	}

	void JitCompilerX86::h_FSUB_M(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		instr.dst %= RegisterCountFlt;
		output.genAddressReg(instr, true);
		output.emit(REX_CVTDQ2PD_XMM12);
		output.emit(REX_SUBPD);
		output.emitByte(0xc4 + 8 * instr.dst);
	}

	void JitCompilerX86::h_FSCAL_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		instr.dst %= RegisterCountFlt;
		output.emit(REX_XORPS);
		output.emitByte(0xc7 + 8 * instr.dst);
	}

	void JitCompilerX86::h_FMUL_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		instr.dst %= RegisterCountFlt;
		instr.src %= RegisterCountFlt;
		output.emit(REX_MULPD);
		output.emitByte(0xe0 + instr.src + 8 * instr.dst);
	}

	void JitCompilerX86::h_FDIV_M(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		instr.dst %= RegisterCountFlt;
		output.genAddressReg(instr, true);
		output.emit(REX_CVTDQ2PD_XMM12);
		output.emit(REX_ANDPS_XMM12);
		output.emit(REX_DIVPD);
		output.emitByte(0xe4 + 8 * instr.dst);
	}

	void JitCompilerX86::h_FSQRT_R(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		instr.dst %= RegisterCountFlt;
		output.emit(SQRTPD);
		output.emitByte(0xe4 + 9 * instr.dst);
	}

	void JitCompilerX86::h_CFROUND(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		if (tuneRounding && !(vmFlags & RANDOMX_FLAG_V2)) {
			// Select an immutable MXCSR value. Avoid the store-to-load dependency
			// of constructing a control word immediately before LDMXCSR.
			const int rotate = (2 - (instr.getImm32() & 63)) & 63;
			if (hasBmi2) {
				const uint8_t rotateRight[] = {0xc4, 0xc3, 0xfb, 0xf0};
				output.emit(rotateRight);
				output.emitByte(0xc0 + instr.src);
				output.emitByte((64 - rotate) & 63);
			} else {
				output.emit(REX_MOV_RR64);
				output.emitByte(0xc0 + instr.src);
				if (rotate) {
					output.emit(ROL_RAX);
					output.emitByte(rotate);
				}
			}
			const uint8_t mask[] = {0x83, 0xe0, 0x0c}; // and eax, 12
			output.emit(mask);
			// Compare only the rounding selector. Exception status bits do not
			// affect RandomX arithmetic because all exceptions are masked.
			const uint8_t compare[] = {0x3b, 0x44, 0x24, 0x04, 0x74};
			output.emit(compare); // cmp eax,[rsp+4]; je end
			const int32_t skipOffset = output.pos;
			output.emitByte(0);
			const uint8_t remember[] = {0x89, 0x44, 0x24, 0x04};
			if (vmFlags & RANDOMX_FLAG_FULL_MEM) {
				const uint8_t load[] = {0x0f, 0xae, 0x54, 0x04, 0x20};
				output.emit(load); // ldmxcsr [rsp+rax+32]
			} else {
				// Light VM calls overwrite the spill area; use its code table.
				const uint8_t address[] = {0x48, 0x8d, 0x0d};
				output.emit(address);
				output.emit32(roundingTableOffset - (output.pos + 4));
				const uint8_t load[] = {0x0f, 0xae, 0x14, 0x01};
				output.emit(load);
			}
			output.emitByte(0xeb);
			output.emitByte(0);
			// Keep the cache store after the control-register load, avoiding a
			// preceding store in this sequence. This is a Zen 2 tuning choice.
			output.emit(remember);
			output.code[skipOffset] = static_cast<uint8_t>(output.pos - skipOffset - 1);
			return;
		}
		//mov rax {src}
		output.emit(REX_MOV_RR64);
		output.emitByte(0xc0 + instr.src);
		int rotate = (13 - (instr.getImm32() & 63)) & 63;
		if (rotate != 0) {
			//rol rax, {imm32}
			output.emit(ROL_RAX);
			output.emitByte(rotate);
		}
		if (vmFlags & RANDOMX_FLAG_V2) {
			//test eax, 491520
			//jnz next
			output.emit(TEST_EAX_60SL13);
			output.emitByte(JNZ_SHORT);
			output.emitByte(sizeof(AND_OR_MOV_LDMXCSR));
		}
		//and eax, 24576
		//or eax, 40896
		//mov dword ptr [rsp], eax
		//ldmxcsr dword ptr [rsp]
		output.emit(AND_OR_MOV_LDMXCSR);
	}

	void JitCompilerX86::h_CBRANCH(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		int reg = instr.dst;
		int target = registerUsage[reg] + 1;
		output.emit(REX_ADD_I);
		output.emitByte(0xc0 + reg);
		int shift = instr.getModCond() + ConditionOffset;
		uint32_t imm = instr.getImm32() | (1UL << shift);
		if (ConditionOffset > 0 || shift > 0)
			imm &= ~(1UL << (shift - 1));
		output.emit32(imm);
		output.emit(REX_TEST);
		output.emitByte(0xc0 + reg);
		output.emit32(ConditionMask << shift);
		const int32_t shortOffset = instructionOffsets[target] - (output.pos + 2);
		if (shortOffset >= -128 && shortOffset <= 127) {
			output.emitByte(0x74); // jz rel8
			output.emitByte(static_cast<uint8_t>(shortOffset));
		} else {
			output.emit(JZ);
			output.emit32(instructionOffsets[target] - (output.pos + 4));
		}
		//mark all registers as used
		for (unsigned j = 0; j < RegistersCount; ++j) {
			registerUsage[j] = i;
		}
	}

	void JitCompilerX86::h_ISTORE(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		output.genAddressRegDst(instr);
		output.emit(REX_MOV_MR);
		output.emitByte(0x04 + 8 * instr.src);
		output.emitByte(0x06);
	}

	void JitCompilerX86::h_NOP(Instruction& instr, int i) {
		InstructionEmitter output(code, codePos);
		output.emit(NOP1);
	}

#include "instruction_weights.hpp"
#define INST_HANDLE(x) REPN(&JitCompilerX86::dispatch<&JitCompilerX86::h_##x>, WT(x))

	const InstructionGeneratorX86 JitCompilerX86::engine[256] = {
		INST_HANDLE(IADD_RS)
		INST_HANDLE(IADD_M)
		INST_HANDLE(ISUB_R)
		INST_HANDLE(ISUB_M)
		INST_HANDLE(IMUL_R)
		INST_HANDLE(IMUL_M)
		INST_HANDLE(IMULH_R)
		INST_HANDLE(IMULH_M)
		INST_HANDLE(ISMULH_R)
		INST_HANDLE(ISMULH_M)
		INST_HANDLE(IMUL_RCP)
		INST_HANDLE(INEG_R)
		INST_HANDLE(IXOR_R)
		INST_HANDLE(IXOR_M)
		INST_HANDLE(IROR_R)
		INST_HANDLE(IROL_R)
		INST_HANDLE(ISWAP_R)
		INST_HANDLE(FSWAP_R)
		INST_HANDLE(FADD_R)
		INST_HANDLE(FADD_M)
		INST_HANDLE(FSUB_R)
		INST_HANDLE(FSUB_M)
		INST_HANDLE(FSCAL_R)
		INST_HANDLE(FMUL_R)
		INST_HANDLE(FDIV_M)
		INST_HANDLE(FSQRT_R)
		INST_HANDLE(CBRANCH)
		INST_HANDLE(CFROUND)
		INST_HANDLE(ISTORE)
		INST_HANDLE(NOP)
	};
}
