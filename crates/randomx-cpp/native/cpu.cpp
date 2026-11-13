/*
Copyright (c) 2019, tevador <tevador@gmail.com>

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

#include "cpu.hpp"
#if defined(_M_X64) || defined(__x86_64__)
#if defined(_MSC_VER)
#include <intrin.h>
static void cpuid(int info[4], int leaf) { __cpuidex(info, leaf, 0); }
#else
#include <cpuid.h>
static void cpuid(int info[4], int leaf) { __cpuid_count(leaf, 0, info[0], info[1], info[2], info[3]); }
#endif
	static bool avxStateEnabled(int features) {
		if ((features & (3 << 27)) != (3 << 27)) return false;
#if defined(_MSC_VER)
		return (_xgetbv(0) & 6) == 6;
#else
		unsigned lo, hi;
		__asm__ volatile ("xgetbv" : "=a"(lo), "=d"(hi) : "c"(0));
		return (lo & 6) == 6;
#endif
	}
#elif defined(__aarch64__) && defined(__linux__)
#include <sys/auxv.h>
#include <asm/hwcap.h>
#endif

namespace randomx {
	Cpu::Cpu() {
#if defined(_M_X64) || defined(__x86_64__)
		int info[4];
		cpuid(info, 0);
		int nIds = info[0];
		bool avxState = false;
		if (nIds >= 0x00000001) {
			cpuid(info, 0x00000001);
			ssse3_ = (info[2] & (1 << 9)) != 0;
			aes_ = (info[2] & (1 << 25)) != 0;
			avxState = avxStateEnabled(info[2]);
		}
		if (nIds >= 0x00000007) {
			cpuid(info, 0x00000007);
			avx2_ = avxState && (info[1] & (1 << 5)) != 0;
			bmi2_ = (info[1] & (1 << 8)) != 0;
			vaes_ = avx2_ && (info[2] & (1 << 9)) != 0;
		}
#elif defined(__aarch64__) && defined(__linux__)
        aes_ = (getauxval(AT_HWCAP) & HWCAP_AES) != 0;
#elif defined(__aarch64__) && defined(__APPLE__)
        aes_ = true;
#endif
	}

	const Cpu cpu;
}
