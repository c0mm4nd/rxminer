// SPDX-License-Identifier: BSD-3-Clause
// Full cache, AES, floating-point and hash oracle for the standalone Rust VM.
#include <cstddef>
#include "randomx.h"
#include "dataset.hpp"
#include "aes_hash.hpp"
#include "soft_aes.h"
#include "blake2/blake2.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <vector>

static FILE* output;
static uint64_t rng = UINT64_C(0x349ea9206ae56d17);
static uint64_t next() { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; return rng; }
static void bytes(const void* p, size_t n) { if (std::fwrite(p, 1, n, output) != n) std::abort(); }
static void u32(uint32_t n) { uint8_t p[4]; for (unsigned i=0;i<4;++i) p[i]=uint8_t(n>>(8*i)); bytes(p,4); }
static void u64(uint64_t n) { u32(uint32_t(n)); u32(uint32_t(n>>32)); }
struct Buffer {
    std::unique_ptr<uint8_t[]> owner;
    uint8_t* data;
    explicit Buffer(size_t size): owner(new uint8_t[size+63]),
        data(reinterpret_cast<uint8_t*>((reinterpret_cast<uintptr_t>(owner.get())+63)&~uintptr_t(63))) {}
};
static void floating() {
    const uint64_t edges[] = {0, UINT64_C(0x8000000000000000), UINT64_C(0x3ff0000000000000),
        UINT64_C(0xbff0000000000000), UINT64_C(0x3ff0000000000001), UINT64_C(0x3fefffffffffffff),
        UINT64_C(0x3ca0000000000000), UINT64_C(0xbca0000000000000), UINT64_C(0x3000000000000000),
        UINT64_C(0x300fffffffffffff), UINT64_C(0x41e0000000000000), UINT64_C(0xc1e0000000000000),
        UINT64_C(0x4000000000000000), UINT64_C(0x3fe0000000000000), UINT64_C(0x403fffffffffffff),
        UINT64_C(0xc03fffffffffffff)};
    u32(4096 * 4);
    for (unsigned mode=0;mode<4;++mode) for(unsigned i=0;i<4096;++i) {
        // The exponent range includes RandomX's F/E registers and keeps
        // subnormal inputs/results outside this FTZ/DAZ hardware comparison.
        const uint64_t a = i<224 ? edges[i%16] : (next() & UINT64_C(0x800fffffffffffff)) | ((768 + (next() & 511)) << 52);
        const uint64_t b = i<224 ? edges[2+i/16] : (next() & UINT64_C(0x800fffffffffffff)) | ((768 + (next() & 511)) << 52);
        u32(mode); u64(a); u64(b);
        rx_set_rounding_mode(mode);
        const auto x=rx_set_vec_f128(a,a), y=rx_set_vec_f128(b,b);
        alignas(16) double result[2]; uint64_t bits;
        rx_store_vec_f128(result,rx_add_vec_f128(x,y)); std::memcpy(&bits,result,8); u64(bits);
        rx_store_vec_f128(result,rx_sub_vec_f128(x,y)); std::memcpy(&bits,result,8); u64(bits);
        rx_store_vec_f128(result,rx_mul_vec_f128(x,y)); std::memcpy(&bits,result,8); u64(bits);
        rx_store_vec_f128(result,rx_div_vec_f128(x,y)); std::memcpy(&bits,result,8); u64(bits);
        const uint64_t positive=a & UINT64_C(0x7fffffffffffffff);
        rx_store_vec_f128(result,rx_sqrt_vec_f128(rx_set_vec_f128(positive,positive)));
        std::memcpy(&bits,result,8); u64(bits);
    }
    rx_reset_float_state();
}
static void aesCases() {
    u32(4096);
    for(unsigned i=0;i<4096;++i) {
        alignas(16) uint8_t state[16], key[16], result[16];
        for(unsigned j=0;j<16;++j) { state[j]=uint8_t(next()); key[j]=uint8_t(next()); }
        bytes(state,16); bytes(key,16);
        auto x=rx_load_vec_i128(reinterpret_cast<rx_vec_i128*>(state));
        auto k=rx_load_vec_i128(reinterpret_cast<rx_vec_i128*>(key));
        rx_store_vec_i128(reinterpret_cast<rx_vec_i128*>(result),aesenc<true>(x,k)); bytes(result,16);
        rx_store_vec_i128(reinterpret_cast<rx_vec_i128*>(result),aesdec<true>(x,k)); bytes(result,16);
    }
    const unsigned sizes[] = {0,64,128,192,2176,3200,4096,2097152};
    u32(8);
    for(unsigned size:sizes) {
        Buffer buffer(size);
        alignas(64) uint8_t state[64], seed[64], hash[64];
        for(auto& b:seed) b=uint8_t(next());
        std::memcpy(state,seed,64);
        u32(size); bytes(seed,64);
        fillAes1Rx4<true>(state,size,buffer.data); bytes(state,64); bytes(buffer.data,size);
        hashAes1Rx4<true>(buffer.data,size,hash); bytes(hash,64);
        fillAes4Rx4<true>(seed,size,buffer.data); bytes(buffer.data,size);
    }
}
static void cacheAndHashes() {
    auto* cache = randomx_alloc_cache(RANDOMX_FLAG_DEFAULT);
    if(!cache) std::abort();
    u32(4);
    for(unsigned caseId=0;caseId<4;++caseId) {
        std::vector<uint8_t> key;
        if(caseId<2) { const char* text=caseId ? "test key 001" : "test key 000"; key.assign(text,text+12); }
        if(caseId==2) { key.resize(80); for(auto& b:key) b=uint8_t(next()); }
        u32(uint32_t(key.size())); bytes(key.data(),key.size());
        randomx_init_cache(cache,key.data(),key.size());
        alignas(64) uint8_t digest[64];
        blake2b(digest,64,randomx_get_cache_memory(cache),randomx::CacheSize,nullptr,0); bytes(digest,64);
        u32(1024);
        for(unsigned i=0;i<1024;++i) {
            uint64_t item = i<4 ? (i==0 ? 0 : i==1 ? 34078718 : i==2 ? UINT64_MAX : UINT64_C(1)<<32) : next();
            alignas(64) uint64_t words[8];
            randomx::initDatasetItem(cache,reinterpret_cast<uint8_t*>(words),item);
            u64(item); for(auto word:words) u64(word);
        }
        u32(6);
        for(unsigned v2=0;v2<2;++v2) {
            auto* vm=randomx_create_vm(v2 ? RANDOMX_FLAG_V2 : RANDOMX_FLAG_DEFAULT,cache,nullptr);
            if(!vm) std::abort();
            for(unsigned size:{0u,76u,257u}) {
                std::vector<uint8_t> input(size);
                for(auto& b:input) b=uint8_t(next());
                u32(v2); u32(size); bytes(input.data(),size);
                randomx_calculate_hash(vm,input.data(),size,digest); bytes(digest,32);
            }
            randomx_destroy_vm(vm);
        }
    }
    randomx_release_cache(cache);
}
int main(int argc,char** argv) {
    if(argc!=2 || !(output=std::fopen(argv[1],"wb"))) return 1;
    bytes("RXVM0001",8);
    floating(); aesCases(); cacheAndHashes();
    if(std::fclose(output)!=0) return 1;
    std::puts("Generated Rust VM oracle: floating point, AES, four cache keys and 24 complete hashes");
}
