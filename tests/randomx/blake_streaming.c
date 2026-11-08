/* SPDX-License-Identifier: BSD-3-Clause
 * Same compression kernel and build flags, without one-shot shortcuts.
 * This reference is linked only into the native regression executable. */
#define RXM_BLAKE2_STREAMING
#define randomx_blake2b_init_param rxm_streaming_init_param
#define randomx_blake2b_init rxm_streaming_init
#define randomx_blake2b_init_key rxm_streaming_init_key
#define randomx_blake2b_update rxm_streaming_update
#define randomx_blake2b_final rxm_streaming_final
#define randomx_blake2b rxm_streaming_blake2b
#define randomx_blake2b_long rxm_streaming_long
#include "blake2/blake2b.c"
