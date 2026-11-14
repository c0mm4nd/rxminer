// SPDX-License-Identifier: BSD-3-Clause
// Rust ownership boundary for the bundled RandomX implementation.
#include "randomx.h"
#include "virtual_machine.hpp"
#include <cfenv>
#if defined(__SSE2__) || defined(_M_X64)
#include <xmmintrin.h>
#endif

namespace {
struct FloatState {
#if defined(__SSE2__) || defined(_M_X64)
    // USE_CSR_INTRINSICS is local to randomx.cpp, not a build definition.
    // SSE2 RandomX changes only MXCSR; avoid saving/restoring the unrelated
    // x87 environment through libc on every pipelined hash.
    unsigned int saved = _mm_getcsr();
    ~FloatState() { _mm_setcsr(saved); }
#else
    fenv_t saved;
    FloatState() { fegetenv(&saved); }
    ~FloatState() { fesetenv(&saved); }
#endif
};
void version(randomx_vm* vm, bool v2) {
    if (v2) vm->setFlagV2(); else vm->clearFlagV2();
}
}
extern "C" bool rxm_hash_next(randomx_vm* vm, const void* input, size_t size, void* output, bool v2) noexcept {
    FloatState guard;
    try { version(vm, v2); randomx_calculate_hash_next(vm, input, size, output); return true; }
    catch (...) { return false; }
}
extern "C" bool rxm_hash_last(randomx_vm* vm, void* output, bool v2) noexcept {
    FloatState guard;
    try { version(vm, v2); randomx_calculate_hash_last(vm, output); return true; }
    catch (...) { return false; }
}
extern "C" bool rxm_init_cache(randomx_cache* cache, const void* key, size_t len) noexcept {
    try { randomx_init_cache(cache, key, len); return true; }
    catch (...) { return false; }
}
extern "C" bool rxm_init_dataset(randomx_dataset* data, randomx_cache* cache, unsigned long start, unsigned long count) noexcept {
    try { randomx_init_dataset(data, cache, start, count); return true; }
    catch (...) { return false; }
}
