// SPDX-License-Identifier: MIT
//! Owned wrappers for the integrated RandomX C ABI. Native pointers never leave
//! this module; VMs are thread-local and keep their dataset alive with an Arc.
use anyhow::{ensure, Context, Result};
use std::{
    ffi::{c_uint, c_ulong, c_void},
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
};

const LARGE_PAGES: u32 = 1;
const HARD_AES: u32 = 2;
const FULL_MEM: u32 = 4;
const JIT: u32 = 8;
const SECURE: u32 = 16;
const ARGON2: u32 = 96;

// Each claim covers at most 4 MiB (65,536 dataset items). Faster initializer
// cores can take more work instead of waiting for a static partition on a slow
// core. The small number of claims keeps coordination off the inner item loop.
fn claim_dataset_chunk(next: &AtomicU64, items: u64) -> Option<(u64, u64)> {
    const CHUNK_ITEMS: u64 = 1 << 16;
    let start = next.fetch_add(CHUNK_ITEMS, Ordering::Relaxed);
    (start < items).then(|| (start, (items - start).min(CHUNK_ITEMS)))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MemoryMode {
    Auto,
    #[default]
    Fast,
    Light,
}

#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    pub mode: MemoryMode,
    pub hw_aes: Option<bool>,
    pub argon2: Option<u32>,
    pub one_gb_pages: bool,
    pub jit: bool,
    pub secure: bool,
}
pub const BACKEND: &str = "C++";
pub const JIT_SUPPORTED: bool = cfg!(any(
    all(any(unix, windows), target_arch = "x86_64"),
    all(unix, target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "riscv64")
));
impl Default for Tuning {
    fn default() -> Self {
        Self {
            mode: MemoryMode::Fast,
            hw_aes: None,
            argon2: None,
            one_gb_pages: false,
            jit: JIT_SUPPORTED,
            secure: false,
        }
    }
}
impl Tuning {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.jit || (JIT_SUPPORTED && flags() & JIT != 0),
            "JIT is unavailable on this platform"
        );
        ensure!(
            self.hw_aes != Some(true) || flags() & HARD_AES != 0,
            "hardware AES is unavailable"
        );
        if let Some(value) = self.argon2 {
            ensure!(
                matches!(value, 0 | 32 | 64),
                "invalid Argon2 implementation"
            );
            ensure!(value != 32 || flags() & 32 != 0, "SSSE3 is unavailable");
            ensure!(value != 64 || flags() & 64 != 0, "AVX2 is unavailable");
        }
        Ok(())
    }
}

pub fn set_memory_pool(pages: usize) {
    // SAFETY: native mutex protects idle mappings; active VM mappings are not freed.
    unsafe {
        rxm_set_memory_pool(pages);
    }
}

pub fn set_allocator_node(node: Option<usize>) {
    // SAFETY: updates only calling thread's allocator locality key.
    unsafe {
        rxm_set_allocator_node(node.map_or(-1, |n| n as i32));
    }
}

pub fn configure_jit(huge_pages: bool, prefetch_mode: u8) {
    assert!(prefetch_mode <= 3, "prefetch mode must be 0..3");
    // SAFETY: atomic native policies; each JIT records its own mapping length and mode.
    unsafe {
        rxm_set_jit_huge_pages(i32::from(huge_pages));
        rxm_set_prefetch_mode(i32::from(prefetch_mode));
    }
}

pub fn set_thp(enabled: bool) {
    // SAFETY: the native setter atomically updates allocation policy; existing allocations are untouched.
    unsafe {
        rxm_set_thp(i32::from(enabled));
    }
}

unsafe extern "C" {
    fn rxm_set_memory_pool(pages: usize);
    fn rxm_set_allocator_node(node: libc::c_int);
    fn rxm_set_thp(enabled: libc::c_int);
    fn rxm_set_jit_huge_pages(enabled: libc::c_int);
    fn rxm_set_prefetch_mode(mode: libc::c_int);
    fn randomx_get_flags() -> c_uint;
    fn randomx_alloc_cache(flags: c_uint) -> *mut c_void;
    fn rxm_init_cache(cache: *mut c_void, key: *const c_void, len: usize) -> bool;
    fn randomx_release_cache(cache: *mut c_void);
    fn randomx_alloc_dataset(flags: c_uint) -> *mut c_void;
    fn randomx_dataset_item_count() -> c_ulong;
    fn rxm_init_dataset(
        dataset: *mut c_void,
        cache: *mut c_void,
        start: c_ulong,
        count: c_ulong,
    ) -> bool;
    fn randomx_release_dataset(dataset: *mut c_void);
    fn randomx_create_vm(flags: c_uint, cache: *mut c_void, dataset: *mut c_void) -> *mut c_void;
    fn randomx_destroy_vm(vm: *mut c_void);
    fn randomx_calculate_hash_first(vm: *mut c_void, input: *const c_void, size: usize);
    fn rxm_hash_next(
        vm: *mut c_void,
        input: *const c_void,
        size: usize,
        output: *mut u8,
        v2: bool,
    ) -> bool;
    fn rxm_hash_last(vm: *mut c_void, output: *mut u8, v2: bool) -> bool;
    fn randomx_calculate_commitment(
        input: *const u8,
        size: usize,
        hash: *const u8,
        output: *mut u8,
    );

}

fn flags() -> u32 {
    // SAFETY: this queries CPU capabilities and does not dereference caller data.
    unsafe { randomx_get_flags() }
}

struct Cache(NonNull<c_void>, bool);
// SAFETY: Cache is initialized before sharing. Dataset initialization only
// reads it; no cache mutation occurs after Cache::new returns.
unsafe impl Send for Cache {}
// SAFETY: see above; concurrent disjoint dataset initialization is supported
// by RandomX's C API and the cache outlives all scoped initializer threads.
unsafe impl Sync for Cache {}

impl Cache {
    fn with_tuning(seed: &[u8], tuning: Tuning, huge: bool) -> Result<Self> {
        tuning.validate()?;
        // Cache JIT accelerates full dataset construction only. Light VMs
        // compile their own superscalar code (or interpret cache.programs).
        let cache_jit = if tuning.mode == MemoryMode::Light || !tuning.jit {
            0
        } else {
            flags() & (JIT | SECURE)
        };
        let mut flags = cache_jit
            | tuning.argon2.unwrap_or(flags() & ARGON2)
            | if huge { LARGE_PAGES } else { 0 };
        // SAFETY: flags are from RandomX's own capability detection.
        let mut ptr = unsafe { randomx_alloc_cache(flags) };
        if ptr.is_null() && flags & LARGE_PAGES != 0 {
            flags &= !LARGE_PAGES;
            // SAFETY: retry the same cache settings with ordinary pages.
            ptr = unsafe { randomx_alloc_cache(flags) };
        }
        if ptr.is_null() && flags & JIT != 0 {
            // SAFETY: retry without optional JIT support on hardened systems.
            ptr = unsafe { randomx_alloc_cache(flags & !JIT) };
        }
        let cache = Self(
            NonNull::new(ptr).context("allocate RandomX cache")?,
            flags & LARGE_PAGES != 0,
        );
        // SAFETY: seed is valid for len bytes; cache is owned and not yet shared.
        unsafe {
            ensure!(
                rxm_init_cache(cache.0.as_ptr(), seed.as_ptr().cast(), seed.len()),
                "initialize RandomX cache"
            );
        }
        Ok(cache)
    }
}
impl Drop for Cache {
    fn drop(&mut self) {
        // SAFETY: unique owner; any scoped initializer or VM has already exited.
        unsafe {
            randomx_release_cache(self.0.as_ptr());
        }
    }
}

pub struct Dataset {
    ptr: Option<NonNull<c_void>>,
    cache: Option<Arc<Cache>>,
    tuning: Tuning,
    huge_pages_requested: bool,
    giga_pages: bool,
    huge_pages: bool,
}
// SAFETY: construction writes disjoint ranges in scoped threads and joins
// them before publishing the Dataset; all subsequent access is read-only.
unsafe impl Send for Dataset {}
// SAFETY: the dataset is immutable after build; VMs hold shared ownership.
unsafe impl Sync for Dataset {}

impl Dataset {
    pub fn is_light(&self) -> bool {
        self.cache.is_some()
    }

    /// Build immutable seed data, configuring allocator and worker placement
    /// through the caller's setup callback before touching memory.
    pub fn build_with_tuning(
        seed: &[u8],
        threads: usize,
        stop: &AtomicBool,
        huge_pages: bool,
        tuning: Tuning,
        setup: impl Fn(usize) -> Result<()> + Sync,
    ) -> Result<Self> {
        tuning.validate()?;
        ensure!(
            threads > 0 && !stop.load(Ordering::Relaxed),
            "initialization interrupted or no CPUs"
        );
        setup(0)?;
        // The initialized cache belongs to this attempt, not to the full-mode
        // branch. If allocation or an initializer fails, light mode can take
        // ownership without another 256 MiB allocation and cache initialization.
        let cache = Cache::with_tuning(seed, tuning, huge_pages)?;
        ensure!(!stop.load(Ordering::Relaxed), "initialization interrupted");
        if tuning.mode != MemoryMode::Light {
            match Self::build_full(&cache, threads, stop, huge_pages, tuning, &setup) {
                Ok(dataset) => return Ok(dataset),
                Err(_error) if tuning.mode == MemoryMode::Auto && !stop.load(Ordering::Relaxed) => {
                }
                Err(error) => return Err(error),
            }
        }
        ensure!(!stop.load(Ordering::Relaxed), "initialization interrupted");
        let cache_huge = cache.1;
        Ok(Self {
            ptr: None,
            cache: Some(Arc::new(cache)),
            tuning,
            huge_pages_requested: huge_pages,
            giga_pages: false,
            huge_pages: cache_huge,
        })
    }

    pub fn uses_huge_pages(&self) -> bool {
        self.huge_pages
    }

    pub fn uses_giga_pages(&self) -> bool {
        self.giga_pages
    }

    fn build_full(
        cache: &Cache,
        threads: usize,
        stop: &AtomicBool,
        huge_pages: bool,
        tuning: Tuning,
        setup: &(impl Fn(usize) -> Result<()> + Sync),
    ) -> Result<Self> {
        ensure!(threads > 0, "dataset initialization requires CPUs");
        ensure!(!stop.load(Ordering::Relaxed), "initialization interrupted");
        let mut allocation_flags = if tuning.one_gb_pages {
            256
        } else if huge_pages {
            LARGE_PAGES
        } else {
            0
        };
        // SAFETY: supported allocation flag; the returned dataset is owned here.
        let mut ptr = unsafe { randomx_alloc_dataset(allocation_flags) };
        if ptr.is_null() && allocation_flags == 256 {
            allocation_flags = if huge_pages { LARGE_PAGES } else { 0 };
            // SAFETY: retry with supported smaller-page allocation flags.
            ptr = unsafe { randomx_alloc_dataset(allocation_flags) };
        }
        if ptr.is_null() && allocation_flags == LARGE_PAGES {
            allocation_flags = 0;
            // SAFETY: retry with the integrated THP-aware ordinary allocator.
            ptr = unsafe { randomx_alloc_dataset(allocation_flags) };
        }
        let dataset = Self {
            ptr: Some(NonNull::new(ptr).context("allocate RandomX dataset")?),
            cache: None,
            tuning,
            huge_pages_requested: huge_pages,
            giga_pages: allocation_flags == 256,
            huge_pages: allocation_flags & LARGE_PAGES != 0,
        };
        #[allow(clippy::unnecessary_cast)] // C unsigned long is 32-bit on Windows.
        // SAFETY: constant item count of the integrated implementation.
        let items = unsafe { randomx_dataset_item_count() } as u64;
        let next = AtomicU64::new(0);
        thread::scope(|scope| -> Result<()> {
            let mut handles = Vec::new();
            for worker in 0..threads {
                let dataset = &dataset;
                let next = &next;
                handles.push(
                    thread::Builder::new()
                        .name(format!("rx-init-{worker}"))
                        .spawn_scoped(scope, move || -> Result<()> {
                            setup(worker)?;
                            loop {
                                ensure!(
                                    !stop.load(Ordering::Relaxed),
                                    "initialization interrupted"
                                );
                                let Some((start, count)) = claim_dataset_chunk(next, items) else {
                                    break;
                                };
                                // SAFETY: atomic claims are disjoint and in bounds.
                                // Both allocations remain alive until every initializer
                                // has joined; the dataset is not published before then.
                                #[allow(clippy::unnecessary_cast)]
                                // Bounds are below the c_ulong item count.
                                unsafe {
                                    ensure!(
                                        rxm_init_dataset(
                                            dataset.ptr.expect("full dataset").as_ptr(),
                                            cache.0.as_ptr(),
                                            start as c_ulong,
                                            count as c_ulong,
                                        ),
                                        "initialize RandomX dataset chunk"
                                    );
                                }
                            }
                            Ok(())
                        })
                        .context("spawn dataset initializer")?,
                );
            }
            for handle in handles {
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("dataset initializer panicked"))??;
            }
            Ok(())
        })?;
        // Scoped initializers have joined; the caller can release the cache
        // on success or retain it for a light-mode fallback on failure.
        Ok(dataset)
    }
}
impl Drop for Dataset {
    fn drop(&mut self) {
        // SAFETY: last owner; no VM or initializer can still reference it.
        unsafe {
            if let Some(ptr) = self.ptr {
                randomx_release_dataset(ptr.as_ptr());
            }
        }
    }
}

/// A VM owns thread-local execution state and retains its immutable seed data.
/// ```compile_fail
/// fn requires_send<T: Send>() {}
/// requires_send::<randomx_cpp::Vm>();
/// ```
pub struct Vm {
    ptr: NonNull<c_void>,
    dataset: Arc<Dataset>,
    tuning: Tuning,
    huge_pages: bool,
    pending: bool,
    poisoned: bool,
    _thread_local: PhantomData<Rc<()>>,
}
impl Vm {
    pub fn new(dataset: Arc<Dataset>) -> Result<Self> {
        let tuning = dataset.tuning;
        Self::create(dataset, tuning)
    }
    /// Create independent execution settings over an existing immutable seed.
    /// The dataset determines full/light memory mode; its cache is not rebuilt.
    pub fn with_tuning(dataset: Arc<Dataset>, tuning: Tuning) -> Result<Self> {
        Self::create(dataset, tuning)
    }
    fn create(dataset: Arc<Dataset>, tuning: Tuning) -> Result<Self> {
        tuning.validate()?;
        let mut options = if tuning.jit {
            JIT | (flags() & SECURE)
        } else {
            0
        };
        if tuning.secure && tuning.jit {
            options |= SECURE;
        }
        if tuning.hw_aes.unwrap_or(flags() & HARD_AES != 0) {
            options |= HARD_AES;
        }
        if !dataset.is_light() {
            options |= FULL_MEM;
        }
        if dataset.huge_pages_requested {
            options |= LARGE_PAGES;
        }
        let cache = dataset
            .cache
            .as_ref()
            .map_or(std::ptr::null_mut(), |c| c.0.as_ptr());
        let data = dataset.ptr.map_or(std::ptr::null_mut(), |p| p.as_ptr());
        // SAFETY: validated flags, correct full/light pointers, both retained by dataset.
        let mut ptr = unsafe { randomx_create_vm(options, cache, data) };
        if ptr.is_null() && options & LARGE_PAGES != 0 {
            options &= !LARGE_PAGES;
            // SAFETY: identical live data, retry only the optional page preference.
            ptr = unsafe { randomx_create_vm(options, cache, data) };
        }
        Ok(Self {
            ptr: NonNull::new(ptr).context("create RandomX VM")?,
            dataset,
            tuning,
            huge_pages: options & LARGE_PAGES != 0,
            pending: false,
            poisoned: false,
            _thread_local: PhantomData,
        })
    }
    pub fn uses_huge_pages(&self) -> bool {
        self.huge_pages
    }
    /// Prepare a replacement before releasing the previous VM. A failed allocation
    /// leaves the old VM usable, and execution settings remain those of this VM.
    pub fn set_dataset(&mut self, dataset: Arc<Dataset>) -> Result<()> {
        ensure!(
            !self.pending && !self.poisoned,
            "cannot replace data while pending or failed"
        );
        ensure!(
            self.dataset.is_light() == dataset.is_light(),
            "replacement must preserve full/light mode"
        );
        *self = Self::create(dataset, self.tuning)?;
        Ok(())
    }
    pub fn first(&mut self, blob: &[u8]) {
        assert!(!self.pending && !self.poisoned, "VM is pending or failed");
        // SAFETY: exclusive live VM and valid input; no input pointer escapes.
        unsafe {
            randomx_calculate_hash_first(self.ptr.as_ptr(), blob.as_ptr().cast(), blob.len());
        }
        self.pending = true;
    }
    pub fn hash(&mut self, blob: &[u8], v2: bool) -> Result<[u8; 32]> {
        self.first(blob);
        self.last_version(v2)
    }
    pub fn next(&mut self, blob: &[u8]) -> Result<[u8; 32]> {
        self.next_version(blob, false)
    }
    pub fn last(&mut self) -> Result<[u8; 32]> {
        self.last_version(false)
    }
    pub fn next_version(&mut self, blob: &[u8], v2: bool) -> Result<[u8; 32]> {
        ensure!(
            self.pending && !self.poisoned,
            "no pending hash or VM failed"
        );
        let mut hash = [0; 32];
        // SAFETY: pending exclusive VM, valid disjoint input/output. The bridge
        // catches C++ exceptions and restores floating-point state on every exit.
        let ok = unsafe {
            rxm_hash_next(
                self.ptr.as_ptr(),
                blob.as_ptr().cast(),
                blob.len(),
                hash.as_mut_ptr(),
                v2,
            )
        };
        self.poisoned = !ok;
        ensure!(ok, "C++ RandomX execution failed");
        Ok(hash)
    }
    pub fn last_version(&mut self, v2: bool) -> Result<[u8; 32]> {
        ensure!(
            self.pending && !self.poisoned,
            "no pending hash or VM failed"
        );
        let mut hash = [0; 32];
        // SAFETY: pending exclusive VM, complete output; bridge restores FP state.
        let ok = unsafe { rxm_hash_last(self.ptr.as_ptr(), hash.as_mut_ptr(), v2) };
        self.poisoned = !ok;
        self.pending = false;
        ensure!(ok, "C++ RandomX execution failed");
        Ok(hash)
    }
}
impl Drop for Vm {
    fn drop(&mut self) {
        // SAFETY: unique VM owner; its retained dataset is dropped afterwards.
        unsafe {
            randomx_destroy_vm(self.ptr.as_ptr());
        }
    }
}
pub fn commitment(input: &[u8], hash: &[u8; 32]) -> [u8; 32] {
    let mut output = [0; 32];
    // SAFETY: complete immutable inputs and disjoint writable 32-byte output.
    unsafe {
        randomx_calculate_commitment(
            input.as_ptr(),
            input.len(),
            hash.as_ptr(),
            output.as_mut_ptr(),
        );
    }
    output
}
