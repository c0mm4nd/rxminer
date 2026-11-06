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

#pragma once

#include <cstdint>
#include <cstring>
#include <vector>
#include "common.hpp"

namespace randomx {

	class Program;
	struct ProgramConfiguration;
	class SuperscalarProgram;
	class JitCompilerX86;
	class Instruction;

	using InstructionGeneratorX86 = void(*)(JitCompilerX86*, Instruction&, int);

	class JitCompilerX86 {
	public:
		JitCompilerX86();
		~JitCompilerX86();
		// The normalized fast path is for programs emitted by fillAes4Rx4Program.
		// Keep the default raw-program path for other compiler clients/tests.
		void generateProgram(Program&, ProgramConfiguration&, bool registersNormalized = false);
		void generateProgramLight(Program&, ProgramConfiguration&, uint32_t, bool registersNormalized = false);
		void generateSuperscalarHash(SuperscalarProgramList &programs, std::vector<uint64_t> &);
		void generateDatasetInitCode();
		ProgramFunc* getProgramFunc() {
			return (ProgramFunc*)code;
		}
		DatasetInitFunc* getDatasetInitFunc() {
			return (DatasetInitFunc*)code;
		}
		uint8_t* getCode() {
			return code;
		}
		size_t getCodeSize();
		void enableWriting();
		void enableExecution();
		void enableAll();

		void setFlags(randomx_flags f) { vmFlags = f; }
	private:
		// Keep one instruction's cursor in a local object. The executable byte
		// buffer cannot alias this local cursor, so code emission need not reload
		// and publish the VM's codePos field after every byte or immediate.
		struct InstructionEmitter {
			uint8_t* const code;
			int32_t pos;
			int32_t& committed;
			InstructionEmitter(uint8_t* buffer, int32_t& cursor)
				: code(buffer), pos(cursor), committed(cursor) {}
			~InstructionEmitter() { committed = pos; }
			InstructionEmitter(const InstructionEmitter&) = delete;
			InstructionEmitter& operator=(const InstructionEmitter&) = delete;
			void emitByte(uint8_t value) { code[pos++] = value; }
			void emit32(uint32_t value) {
				memcpy(code + pos, &value, sizeof(value));
				pos += sizeof(value);
			}
			void emit64(uint64_t value) {
				memcpy(code + pos, &value, sizeof(value));
				pos += sizeof(value);
			}
			template<size_t N> void emit(const uint8_t (&bytes)[N]) { emit(bytes, N); }
			void emit(const uint8_t* bytes, size_t size) {
				memcpy(code + pos, bytes, size);
				pos += size;
			}
			void genAddressReg(Instruction&, bool);
			void genAddressRegDst(Instruction&);
			void genAddressImm(Instruction&);
			void genSIB(int scale, int index, int base) {
				emitByte((scale << 6) | (index << 3) | base);
			}
		};
		template<void (JitCompilerX86::*Handler)(Instruction&, int)>
		static void dispatch(JitCompilerX86* jit, Instruction& instruction, int index) {
			(jit->*Handler)(instruction, index);
		}
		static const InstructionGeneratorX86 engine[256];
		int32_t instructionOffsets[RANDOMX_PROGRAM_MAX_SIZE];
		int registerUsage[RegistersCount];
		uint32_t reciprocalCount = 0;
		int32_t loopStart = 0;
		bool stackReciprocals = false;
		uint8_t* code;
		size_t mappedSize;
		int prefetchMode;
		int32_t codePos;
		bool hasBmi2 = false;
		bool tuneRounding = false;

		randomx_flags vmFlags;

		void generateProgramPrologue(Program&, ProgramConfiguration&, bool);
		void generateProgramEpilogue(Program&, ProgramConfiguration&);
		void genSIB(int scale, int index, int base);

		void generateCode(Instruction&, int);
		void generateSuperscalarCode(Instruction &, std::vector<uint64_t> &);

		void alignCode(int align);

		void emitByte(uint8_t val) {
			code[codePos] = val;
			codePos++;
		}

		void emit32(uint32_t val) {
			memcpy(code + codePos, &val, sizeof val);
			codePos += sizeof val;
		}

		void emit64(uint64_t val) {
			memcpy(code + codePos, &val, sizeof val);
			codePos += sizeof val;
		}

		template<size_t N>
		void emit(const uint8_t (&src)[N]) {
			emit(src, N);
		}

		void emit(const uint8_t* src, size_t count) {
			memcpy(code + codePos, src, count);
			codePos += count;
		}

		void h_IADD_RS(Instruction&, int);
		void h_IADD_M(Instruction&, int);
		void h_ISUB_R(Instruction&, int);
		void h_ISUB_M(Instruction&, int);
		void h_IMUL_R(Instruction&, int);
		void h_IMUL_M(Instruction&, int);
		void h_IMULH_R(Instruction&, int);
		void h_IMULH_M(Instruction&, int);
		void h_ISMULH_R(Instruction&, int);
		void h_ISMULH_M(Instruction&, int);
		void h_IMUL_RCP(Instruction&, int);
		void h_INEG_R(Instruction&, int);
		void h_IXOR_R(Instruction&, int);
		void h_IXOR_M(Instruction&, int);
		void h_IROR_R(Instruction&, int);
		void h_IROL_R(Instruction&, int);
		void h_ISWAP_R(Instruction&, int);
		void h_FSWAP_R(Instruction&, int);
		void h_FADD_R(Instruction&, int);
		void h_FADD_M(Instruction&, int);
		void h_FSUB_R(Instruction&, int);
		void h_FSUB_M(Instruction&, int);
		void h_FSCAL_R(Instruction&, int);
		void h_FMUL_R(Instruction&, int);
		void h_FDIV_M(Instruction&, int);
		void h_FSQRT_R(Instruction&, int);
		void h_CBRANCH(Instruction&, int);
		void h_CFROUND(Instruction&, int);
		void h_ISTORE(Instruction&, int);
		void h_NOP(Instruction&, int);
	};

}
