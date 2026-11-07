/* SPDX-License-Identifier: BSD-3-Clause
 * Compile the retained compact-round implementation separately with the same
 * C compiler and optimization level as Cargo's archive. Never linked into the miner. */
#define RXM_BLAKE2_COMPACT
#define randomx_blake2b_init_param rxm_compact_init_param
#define randomx_blake2b_init rxm_compact_init
#define randomx_blake2b_init_key rxm_compact_init_key
#define randomx_blake2b_update rxm_compact_update
#define randomx_blake2b_final rxm_compact_final
#define randomx_blake2b rxm_compact_blake2b
#define randomx_blake2b_long rxm_compact_long
#include "blake2/blake2b.c"
