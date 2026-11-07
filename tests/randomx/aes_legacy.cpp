// SPDX-License-Identifier: BSD-3-Clause
// Compile the retained round-call/single-block path with Cargo's compiler flags.
// Rename inline templates as well as exported functions to avoid ODR collisions.
#define RXM_AES_LEGACY
#define aesenc rxm_legacy_aesenc
#define aesdec rxm_legacy_aesdec
#define hashAes1Rx4 rxm_legacy_hashAes1Rx4
#define fillAes1Rx4 rxm_legacy_fillAes1Rx4
#define fillAes4Rx4 rxm_legacy_fillAes4Rx4
#define fillAes4Rx4Program rxm_legacy_fillAes4Rx4Program
#define hashAndFillAes1Rx4 rxm_legacy_hashAndFillAes1Rx4
#include "aes_hash.cpp"
