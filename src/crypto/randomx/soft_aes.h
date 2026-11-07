/*
Copyright (c) 2018-2019, tevador <tevador@gmail.com>
Copyright (c) 2019 SChernykh <https://github.com/SChernykh>

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

#include <stdint.h>
#include "intrin_portable.h"

extern "C" const uint32_t randomx_aes_lut_enc[4][256];
extern "C" const uint32_t randomx_aes_lut_dec[4][256];

extern "C" const uint8_t randomx_aes_lut_enc_index[4][32];
extern "C" const uint8_t randomx_aes_lut_dec_index[4][32];

rx_vec_i128 soft_aesenc(rx_vec_i128 in, rx_vec_i128 key);

rx_vec_i128 soft_aesdec(rx_vec_i128 in, rx_vec_i128 key);

#if !defined(RXM_AES_LEGACY) && (defined(__aarch64__) || ((defined(__x86_64__) || defined(_M_X64)) && !defined(__clang__)))
// Inline the table round, following XMRig's BSD-licensed aesenc/aesdec<2>
// (b2ca72480c58d197e18c885d9fc1a0c8d517e60a). Keep the out-of-line API for
// the portable fallback and differential tests. Clang x86 retains calls: both
// inline lookup layouts regressed in same-buffer kernel comparisons. Only
// immutable tables are shared; ARM64 Clang benefits from inlining these rounds.
template<bool decrypt>
FORCE_INLINE rx_vec_i128 rxm_soft_aes_round(rx_vec_i128 in, rx_vec_i128 key) {
	const uint32_t (*lut)[256] = decrypt ? randomx_aes_lut_dec : randomx_aes_lut_enc;
	const uint32_t s0 = rx_vec_i128_w(in), s1 = rx_vec_i128_z(in);
	const uint32_t s2 = rx_vec_i128_y(in), s3 = rx_vec_i128_x(in);
	const rx_vec_i128 out = rx_set_int_vec_i128(
		lut[0][s0 & 255] ^ lut[1][((decrypt ? s1 : s3) >> 8) & 255] ^ lut[2][(s2 >> 16) & 255] ^ lut[3][(decrypt ? s3 : s1) >> 24],
		lut[0][s1 & 255] ^ lut[1][((decrypt ? s2 : s0) >> 8) & 255] ^ lut[2][(s3 >> 16) & 255] ^ lut[3][(decrypt ? s0 : s2) >> 24],
		lut[0][s2 & 255] ^ lut[1][((decrypt ? s3 : s1) >> 8) & 255] ^ lut[2][(s0 >> 16) & 255] ^ lut[3][(decrypt ? s1 : s3) >> 24],
		lut[0][s3 & 255] ^ lut[1][((decrypt ? s0 : s2) >> 8) & 255] ^ lut[2][(s1 >> 16) & 255] ^ lut[3][(decrypt ? s2 : s0) >> 24]);
	return rx_xor_vec_i128(out, key);
}

template<bool soft>
FORCE_INLINE rx_vec_i128 aesenc(rx_vec_i128 in, rx_vec_i128 key) {
	return soft ? rxm_soft_aes_round<false>(in, key) : rx_aesenc_vec_i128(in, key);
}

template<bool soft>
FORCE_INLINE rx_vec_i128 aesdec(rx_vec_i128 in, rx_vec_i128 key) {
	return soft ? rxm_soft_aes_round<true>(in, key) : rx_aesdec_vec_i128(in, key);
}
#else
template<bool soft>
inline rx_vec_i128 aesenc(rx_vec_i128 in, rx_vec_i128 key) {
	return soft ? soft_aesenc(in, key) : rx_aesenc_vec_i128(in, key);
}

template<bool soft>
inline rx_vec_i128 aesdec(rx_vec_i128 in, rx_vec_i128 key) {
	return soft ? soft_aesdec(in, key) : rx_aesdec_vec_i128(in, key);
}
#endif
