/*
Copyright (c) 2018-2019, tevador <tevador@gmail.com>

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

#include <new>
#include <mutex>
#include <vector>
#include "allocator.hpp"
#include "intrin_portable.h"
#include "virtual_memory.h"
#include "common.hpp"

namespace {
    const size_t scratchpadSize = 2UL << 20;
    thread_local int allocationNode = -1;
    struct CachedPage { void* ptr; bool huge; int node; };
    struct ScratchpadPool {
        std::mutex mutex;
        std::vector<CachedPage> pages;
        size_t capacity = 0;
        ~ScratchpadPool() {
            for (const auto& page : pages) freePagedMemory(page.ptr, scratchpadSize);
        }
        void* take(size_t count, bool huge) {
            if (count != scratchpadSize) return nullptr;
            std::lock_guard<std::mutex> guard(mutex);
            for (auto it = pages.begin(); it != pages.end(); ++it) {
                if (it->huge == huge && it->node == allocationNode) {
                    void* ptr = it->ptr;
                    pages.erase(it);
                    return ptr;
                }
            }
            return nullptr;
        }
        bool put(void* ptr, size_t count, bool huge) {
            if (!ptr || count != scratchpadSize) return false;
            std::lock_guard<std::mutex> guard(mutex);
            if (pages.size() >= capacity) return false;
            // Never propagate an allocation failure through a VM destructor.
            try { pages.push_back({ptr, huge, allocationNode}); }
            catch (...) { return false; }
            return true;
        }
    } pool;
}
extern "C" void rxm_set_allocator_node(int node) { allocationNode = node; }
extern "C" void rxm_set_memory_pool(size_t capacity) {
    std::lock_guard<std::mutex> guard(pool.mutex);
    // Drop idle mappings when policy changes; live mappings remain owned by VMs.
    for (const auto& page : pool.pages) freePagedMemory(page.ptr, scratchpadSize);
    pool.pages.clear();
    pool.capacity = capacity;
}

namespace randomx {

	template<size_t alignment>
	void* AlignedAllocator<alignment>::allocMemory(size_t count) {
		/* RxMiner: route through mmap so large allocations are 2 MiB aligned
		 * and eligible for transparent huge pages. Page alignment is a superset
		 * of every alignment RandomX asks for here. */
		void *mem = pool.take(count, false);
        if (!mem) mem = allocMemoryPages(count);
		if (mem == nullptr)
			throw std::bad_alloc();
		return mem;
	}

	template<size_t alignment>
	void AlignedAllocator<alignment>::freeMemory(void* ptr, size_t count) {
		if (!pool.put(ptr, count, false)) freePagedMemory(ptr, count);
	}

	template struct AlignedAllocator<CacheLineSize>;

	void* GigaPageAllocator::allocMemory(size_t count) {
        void* mem = allocGigaPagesMemory(count);
        if (!mem) throw std::bad_alloc();
        return mem;
    }
    void GigaPageAllocator::freeMemory(void* ptr, size_t count) {
        freePagedMemory(ptr, alignSize(count, 1UL << 30));
    }

	void* LargePageAllocator::allocMemory(size_t count) {
		void *mem = pool.take(count, true);
        if (!mem) mem = allocLargePagesMemory(count);
		if (mem == nullptr)
			throw std::bad_alloc();
		return mem;
	}

	void LargePageAllocator::freeMemory(void* ptr, size_t count) {
		if (!pool.put(ptr, count, true)) freePagedMemory(ptr, alignSize(count, 2UL << 20));
	};

}
