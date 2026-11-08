//! Owned wrappers for the integrated RandomX C ABI. Native pointers never leave
//! this module; VMs are thread-local and keep their dataset alive with an Arc.
use crate::topology::{self, Core};
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

#[derive(Clone, Copy, Debug, Default)]
pub struct Tuning {
    pub mode: MemoryMode,
    pub hw_aes: Option<bool>,
    pub argon2: Option<u32>,
    pub one_gb_pages: bool,
}
impl Tuning {
    pub fn validate(&self) -> Result<()> {
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

pub(crate) fn set_allocator_node(node: Option<usize>) {
    // SAFETY: updates only calling thread's allocator locality key.
    unsafe {
        rxm_set_allocator_node(node.map_or(-1, |n| n as i32));
    }
}

pub fn configure_jit(huge_pages: bool, prefetch_mode: u8) {
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
    fn randomx_init_cache(cache: *mut c_void, key: *const c_void, len: usize);
    fn randomx_release_cache(cache: *mut c_void);
    fn randomx_alloc_dataset(flags: c_uint) -> *mut c_void;
    fn randomx_dataset_item_count() -> c_ulong;
    fn randomx_init_dataset(
        dataset: *mut c_void,
        cache: *mut c_void,
        start: c_ulong,
        count: c_ulong,
    );
    fn randomx_release_dataset(dataset: *mut c_void);
    fn randomx_create_vm(flags: c_uint, cache: *mut c_void, dataset: *mut c_void) -> *mut c_void;
    fn randomx_destroy_vm(vm: *mut c_void);
    fn randomx_calculate_hash_first(vm: *mut c_void, input: *const c_void, size: usize);
    fn randomx_calculate_hash_next(
        vm: *mut c_void,
        input: *const c_void,
        size: usize,
        output: *mut c_void,
    );
    fn randomx_calculate_hash_last(vm: *mut c_void, output: *mut c_void);
    #[cfg(test)]
    fn randomx_calculate_hash(
        vm: *mut c_void,
        input: *const c_void,
        size: usize,
        output: *mut c_void,
    );
}

fn flags() -> u32 {
    // SAFETY: this queries CPU capabilities and does not dereference caller data.
    unsafe { randomx_get_flags() }
}

struct Cache(NonNull<c_void>);
#[cfg(test)]
thread_local! {
    static CACHE_INITIALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
// SAFETY: Cache is initialized before sharing. Dataset initialization only
// reads it; no cache mutation occurs after Cache::new returns.
unsafe impl Send for Cache {}
// SAFETY: see above; concurrent disjoint dataset initialization is supported
// by RandomX's C API and the cache outlives all scoped initializer threads.
unsafe impl Sync for Cache {}

impl Cache {
    #[cfg(test)]
    fn new(seed: &[u8]) -> Result<Self> {
        Self::with_tuning(seed, Tuning::default())
    }
    fn with_tuning(seed: &[u8], tuning: Tuning) -> Result<Self> {
        tuning.validate()?;
        // Cache JIT accelerates full dataset construction only. Light VMs
        // compile their own superscalar code (or interpret cache.programs).
        let cache_jit = if tuning.mode == MemoryMode::Light {
            0
        } else {
            flags() & (JIT | SECURE)
        };
        let flags = cache_jit | tuning.argon2.unwrap_or(flags() & ARGON2);
        // SAFETY: flags are from RandomX's own capability detection.
        let mut ptr = unsafe { randomx_alloc_cache(flags) };
        if ptr.is_null() && flags & JIT != 0 {
            // SAFETY: retry without optional JIT support on hardened systems.
            ptr = unsafe { randomx_alloc_cache(flags & !JIT) };
        }
        let cache = Self(NonNull::new(ptr).context("allocate RandomX cache")?);
        // SAFETY: seed is valid for len bytes; cache is owned and not yet shared.
        unsafe {
            randomx_init_cache(cache.0.as_ptr(), seed.as_ptr().cast(), seed.len());
        }
        #[cfg(test)]
        CACHE_INITIALIZATIONS.with(|count| count.set(count.get() + 1));
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

    pub fn build_with_tuning(
        seed: &[u8; 32],
        cores: &[Core],
        node: Option<usize>,
        stop: &AtomicBool,
        huge_pages: bool,
        tuning: Tuning,
    ) -> Result<Self> {
        Self::build_with_fallback(seed, cores, node, stop, huge_pages, tuning, |cache| {
            Self::build_full(cache, cores, node, stop, huge_pages, tuning)
        })
    }

    fn build_with_fallback(
        seed: &[u8; 32],
        cores: &[Core],
        node: Option<usize>,
        stop: &AtomicBool,
        huge_pages: bool,
        tuning: Tuning,
        build_full: impl FnOnce(&Cache) -> Result<Self>,
    ) -> Result<Self> {
        tuning.validate()?;
        ensure!(
            !cores.is_empty() && !stop.load(Ordering::Relaxed),
            "initialization interrupted or no CPUs"
        );
        if cores[0].pinned {
            topology::pin(cores[0].cpu).context("pin cache/dataset allocator")?;
        }
        if let Some(node) = node {
            let _ = topology::bind_node(node);
        }
        // The initialized cache belongs to this attempt, not to the full-mode
        // branch. If allocation or an initializer fails, light mode can take
        // ownership without another 256 MiB allocation and cache initialization.
        let cache = Cache::with_tuning(seed, tuning)?;
        ensure!(!stop.load(Ordering::Relaxed), "initialization interrupted");
        if tuning.mode != MemoryMode::Light {
            match build_full(&cache) {
                Ok(dataset) => return Ok(dataset),
                Err(error) if tuning.mode == MemoryMode::Auto && !stop.load(Ordering::Relaxed) => {
                    crate::status!("[rx] full-memory initialization failed: {error:#}; switching to light mode");
                }
                Err(error) => return Err(error),
            }
        }
        ensure!(!stop.load(Ordering::Relaxed), "initialization interrupted");
        Ok(Self {
            ptr: None,
            cache: Some(Arc::new(cache)),
            tuning,
            huge_pages_requested: huge_pages,
            giga_pages: false,
            huge_pages: false,
        })
    }

    pub fn uses_huge_pages(&self) -> bool {
        self.huge_pages
    }

    pub fn uses_giga_pages(&self) -> bool {
        self.giga_pages
    }

    pub fn build(
        seed: &[u8; 32],
        cores: &[Core],
        node: Option<usize>,
        stop: &AtomicBool,
        huge_pages: bool,
    ) -> Result<Self> {
        Self::build_with_tuning(seed, cores, node, stop, huge_pages, Tuning::default())
    }

    fn build_full(
        cache: &Cache,
        cores: &[Core],
        node: Option<usize>,
        stop: &AtomicBool,
        huge_pages: bool,
        tuning: Tuning,
    ) -> Result<Self> {
        ensure!(!cores.is_empty(), "dataset initialization requires CPUs");
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
            crate::status!("[rx] 1 GiB pages unavailable; trying smaller pages");
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
            for core in cores {
                let dataset = &dataset;
                let next = &next;
                handles.push(
                    thread::Builder::new()
                        .name(format!("rx-init-{}", core.cpu))
                        .spawn_scoped(scope, move || -> Result<()> {
                            if core.pinned {
                                topology::pin(core.cpu).context("pin dataset initializer")?;
                            }
                            if let Some(node) = node {
                                let _ = topology::bind_node(node);
                            }
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
                                    randomx_init_dataset(
                                        dataset.ptr.expect("full dataset").as_ptr(),
                                        cache.0.as_ptr(),
                                        start as c_ulong,
                                        count as c_ulong,
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

/// A VM must remain on its creating thread; only initialized datasets are shared.
///
/// ```compile_fail
/// fn requires_send<T: Send>() {}
/// requires_send::<rxminer::randomx::Vm>();
/// ```
/// ```compile_fail
/// fn requires_sync<T: Sync>() {}
/// requires_sync::<rxminer::randomx::Vm>();
/// ```
pub struct Vm {
    ptr: NonNull<c_void>,
    huge_pages: bool,
    _dataset: Option<Arc<Dataset>>,
    _cache: Option<Arc<Cache>>,
    // RandomX VM floating-point/JIT state belongs to its creating thread.
    _thread_local: PhantomData<Rc<()>>,
    pending: bool,
}

impl Vm {
    pub fn uses_huge_pages(&self) -> bool {
        self.huge_pages
    }

    pub fn new(dataset: Arc<Dataset>) -> Result<Self> {
        Self::create(Some(dataset), None)
    }

    fn create(dataset: Option<Arc<Dataset>>, cache: Option<Arc<Cache>>) -> Result<Self> {
        let mut flags = flags() & (JIT | SECURE | HARD_AES);
        if let Some(data) = &dataset {
            if data.ptr.is_some() {
                flags |= FULL_MEM;
            }
            if let Some(enabled) = data.tuning.hw_aes {
                if enabled {
                    flags |= HARD_AES;
                } else {
                    flags &= !HARD_AES;
                }
            }
        }
        if dataset.as_ref().is_some_and(|d| d.huge_pages_requested) {
            flags |= LARGE_PAGES;
        }
        let dataset_ptr = dataset
            .as_ref()
            .and_then(|d| d.ptr)
            .map_or(std::ptr::null_mut(), |p| p.as_ptr());
        let cache_ptr = cache
            .as_ref()
            .or_else(|| dataset.as_ref().and_then(|d| d.cache.as_ref()))
            .map_or(std::ptr::null_mut(), |c| c.0.as_ptr());
        // SAFETY: matching cache/dataset mode, supported flags, live allocations.
        let mut ptr = unsafe { randomx_create_vm(flags, cache_ptr, dataset_ptr) };
        if ptr.is_null() && flags & LARGE_PAGES != 0 {
            flags &= !LARGE_PAGES;
            // SAFETY: same ownership and mode; preserve JIT before trying the
            // interpreter, since an unavailable huge-page pool is recoverable.
            ptr = unsafe { randomx_create_vm(flags, cache_ptr, dataset_ptr) };
        }
        if ptr.is_null() && flags & JIT != 0 {
            flags &= !JIT;
            // SAFETY: same ownership and mode, falling back to the interpreter.
            ptr = unsafe { randomx_create_vm(flags, cache_ptr, dataset_ptr) };
        }
        Ok(Self {
            ptr: NonNull::new(ptr).context("create RandomX VM")?,
            huge_pages: flags & LARGE_PAGES != 0,
            _dataset: dataset,
            _cache: cache,
            _thread_local: PhantomData,
            pending: false,
        })
    }

    pub fn first(&mut self, blob: &[u8]) {
        assert!(!self.pending);
        // SAFETY: exclusive VM, valid input; the API consumes input during the call.
        unsafe {
            randomx_calculate_hash_first(self.ptr.as_ptr(), blob.as_ptr().cast(), blob.len());
        }
        self.pending = true;
    }

    pub fn next(&mut self, blob: &[u8]) -> [u8; 32] {
        assert!(self.pending);
        let mut hash = [0; 32];
        // SAFETY: first() established the pipeline; output has RANDOMX_HASH_SIZE bytes.
        unsafe {
            randomx_calculate_hash_next(
                self.ptr.as_ptr(),
                blob.as_ptr().cast(),
                blob.len(),
                hash.as_mut_ptr().cast(),
            );
        }
        hash
    }

    pub fn last(&mut self) -> [u8; 32] {
        assert!(self.pending);
        let mut hash = [0; 32];
        // SAFETY: pending hash exists, output is a 32-byte writable buffer.
        unsafe {
            randomx_calculate_hash_last(self.ptr.as_ptr(), hash.as_mut_ptr().cast());
        }
        self.pending = false;
        hash
    }
}
impl Drop for Vm {
    fn drop(&mut self) {
        // SAFETY: exclusive VM owner, dataset/cache fields are dropped afterwards.
        unsafe {
            randomx_destroy_vm(self.ptr.as_ptr());
        }
    }
}

#[cfg(test)]
pub(crate) fn light_hashes(seed: &[u8], blobs: &[Vec<u8>]) -> Vec<[u8; 32]> {
    let cache = Arc::new(Cache::new(seed).unwrap());
    let vm = Vm::create(None, Some(cache)).unwrap();
    blobs
        .iter()
        .map(|blob| {
            let mut hash = [0; 32];
            // SAFETY: exclusive initialized light VM and valid input/output buffers.
            unsafe {
                randomx_calculate_hash(
                    vm.ptr.as_ptr(),
                    blob.as_ptr().cast(),
                    blob.len(),
                    hash.as_mut_ptr().cast(),
                );
            }
            hash
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_fallback_reuses_cache_and_keeps_it_alive_for_vm() {
        let seed = [0x57; 32];
        let blob = b"auto fallback retains the initialized seed".to_vec();
        let expected = light_hashes(&seed, std::slice::from_ref(&blob))[0];
        let cores = [Core {
            cpu: 0,
            node: 0,
            pinned: false,
            smt: false,
        }];
        let stop = AtomicBool::new(false);
        let before = CACHE_INITIALIZATIONS.with(|count| count.get());
        let attempted_cache = std::cell::Cell::new(None);
        let dataset = Dataset::build_with_fallback(
            &seed,
            &cores,
            None,
            &stop,
            true,
            Tuning {
                mode: MemoryMode::Auto,
                hw_aes: Some(false),
                one_gb_pages: true,
                ..Tuning::default()
            },
            |cache| {
                attempted_cache.set(Some(cache.0));
                anyhow::bail!("injected dataset allocation failure")
            },
        )
        .unwrap();
        assert_eq!(CACHE_INITIALIZATIONS.with(|count| count.get()), before + 1);
        assert!(dataset.is_light());
        assert!(!dataset.uses_huge_pages() && !dataset.uses_giga_pages());
        assert!(dataset.huge_pages_requested);
        assert_eq!(dataset.tuning.hw_aes, Some(false));
        let cache = dataset.cache.as_ref().unwrap();
        assert_eq!(Some(cache.0), attempted_cache.get());
        let lifetime = Arc::downgrade(cache);
        let dataset = Arc::new(dataset);
        let mut vm = Vm::new(dataset.clone()).unwrap();
        drop(dataset);
        assert!(lifetime.upgrade().is_some());
        vm.first(&blob);
        assert_eq!(vm.last(), expected);
        drop(vm);
        assert!(lifetime.upgrade().is_none());
    }

    #[test]
    fn fallback_respects_fast_light_and_cancellation() {
        let cores = [Core {
            cpu: 0,
            node: 0,
            pinned: false,
            smt: false,
        }];
        for (mode, cancel) in [
            (MemoryMode::Fast, false),
            (MemoryMode::Light, false),
            (MemoryMode::Auto, true),
        ] {
            let stop = AtomicBool::new(false);
            let before = CACHE_INITIALIZATIONS.with(|count| count.get());
            let attempted = std::cell::Cell::new(false);
            let result = Dataset::build_with_fallback(
                &[0x91; 32],
                &cores,
                None,
                &stop,
                false,
                Tuning {
                    mode,
                    ..Tuning::default()
                },
                |_| {
                    attempted.set(true);
                    stop.store(cancel, Ordering::Relaxed);
                    anyhow::bail!("injected initializer failure")
                },
            );
            assert_eq!(CACHE_INITIALIZATIONS.with(|count| count.get()), before + 1);
            if mode == MemoryMode::Light {
                assert!(!attempted.get());
                assert!(result.unwrap().is_light());
            } else {
                assert!(attempted.get());
                let error = result
                    .err()
                    .expect("fast/cancelled build must not fall back");
                assert!(error.to_string().contains("injected initializer failure"));
            }
        }
        let before = CACHE_INITIALIZATIONS.with(|count| count.get());
        assert!(Dataset::build_with_tuning(
            &[0; 32],
            &cores,
            None,
            &AtomicBool::new(true),
            false,
            Tuning {
                mode: MemoryMode::Auto,
                ..Tuning::default()
            },
        )
        .is_err());
        assert_eq!(CACHE_INITIALIZATIONS.with(|count| count.get()), before);
    }

    #[test]
    fn dataset_chunks_cover_every_item_once() {
        for items in [0, 1, 65_535, 65_536, 65_537, 4_194_321] {
            let next = AtomicU64::new(0);
            let mut chunks = thread::scope(|scope| {
                let workers: Vec<_> = (0..16)
                    .map(|_| {
                        let next = &next;
                        scope.spawn(move || {
                            let mut chunks = Vec::new();
                            while let Some(chunk) = claim_dataset_chunk(next, items) {
                                chunks.push(chunk);
                                thread::yield_now();
                            }
                            chunks
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .flat_map(|worker| worker.join().unwrap())
                    .collect::<Vec<_>>()
            });
            chunks.sort_unstable();
            let mut end = 0;
            for (start, count) in chunks {
                assert_eq!(start, end, "gap or overlapping dataset writes");
                assert!((1..=65_536).contains(&count));
                end += count;
            }
            assert_eq!(end, items);
        }
    }

    #[test]
    fn slow_initializer_does_not_hold_a_static_partition() {
        let items = 65_536 * 8 + 17;
        let next = AtomicU64::new(0);
        thread::scope(|scope| {
            let (claimed_tx, claimed_rx) = std::sync::mpsc::channel();
            let (resume_tx, resume_rx) = std::sync::mpsc::channel();
            let next = &next;
            let slow = scope.spawn(move || {
                let chunk = claim_dataset_chunk(next, items).unwrap();
                claimed_tx.send(chunk).unwrap();
                // Closing the sender also releases this thread on test failure.
                let _ = resume_rx.recv();
                assert!(claim_dataset_chunk(next, items).is_none());
            });
            let (start, mut completed) = claimed_rx.recv().unwrap();
            assert_eq!(start, 0);
            // The available initializer can consume all remaining work while
            // the other one is stalled; no thread owns an unfinished tail.
            while let Some((start, count)) = claim_dataset_chunk(next, items) {
                assert_eq!(start, completed);
                completed += count;
            }
            assert_eq!(completed, items);
            resume_tx.send(()).unwrap();
            slow.join().unwrap();
        });
    }

    #[test]
    fn shared_light_cache_and_native_tuning_preserve_consensus() {
        let expected =
            hex::decode("639183aae1bf4c9a35884cb46b09cad9175f04efd7684e7262a0ac1c2f0b4e3f")
                .unwrap();
        let mut kernels = vec![0];
        if flags() & 32 != 0 {
            kernels.push(32);
        }
        if flags() & 64 != 0 {
            kernels.push(64);
        }
        for argon2 in kernels {
            let tuning = Tuning {
                mode: MemoryMode::Light,
                hw_aes: Some(false),
                argon2: Some(argon2),
                ..Tuning::default()
            };
            let cache = Arc::new(Cache::with_tuning(b"test key 000", tuning).unwrap());
            for mode in 0..=3 {
                configure_jit(true, mode);
                set_memory_pool(4);
                thread::scope(|scope| {
                    for hardware in [false, flags() & HARD_AES != 0] {
                        let cache = cache.clone();
                        let expected = &expected;
                        scope.spawn(move || {
                            let dataset = Arc::new(Dataset {
                                ptr: None,
                                cache: Some(cache),
                                tuning: Tuning {
                                    hw_aes: Some(hardware),
                                    ..tuning
                                },
                                huge_pages_requested: false,
                                huge_pages: false,
                                giga_pages: false,
                            });
                            // Repeated VM creation exercises scratchpad reuse. Each VM stays on its owner thread.
                            for _ in 0..2 {
                                let mut vm = Vm::new(dataset.clone()).unwrap();
                                vm.first(b"This is a test");
                                assert_eq!(vm.last().as_slice(), expected);
                            }
                        });
                    }
                });
            }
        }
        set_memory_pool(0);
        configure_jit(false, 1);
    }

    #[test]
    #[ignore = "allocates a full dataset; exercises 1 GiB fallback and all JIT prefetch modes"]
    fn full_dataset_prefetch_modes_match_light_hashes() {
        let cores = topology::select(&topology::discover().unwrap(), 8, false);
        let seed = [0; 32];
        let blobs = vec![b"prefetch test one".to_vec(), b"prefetch test two".to_vec()];
        let expected = light_hashes(&seed, &blobs);
        let dataset = Arc::new(
            Dataset::build_with_tuning(
                &seed,
                &cores,
                None,
                &AtomicBool::new(false),
                true,
                Tuning {
                    one_gb_pages: true,
                    ..Tuning::default()
                },
            )
            .unwrap(),
        );
        for mode in 0..=3 {
            configure_jit(true, mode);
            let mut vm = Vm::new(dataset.clone()).unwrap();
            vm.first(&blobs[0]);
            assert_eq!(vm.next(&blobs[1]), expected[0], "prefetch mode {mode}");
            assert_eq!(vm.last(), expected[1], "prefetch mode {mode}");
        }
        configure_jit(false, 1);
    }

    #[test]
    #[ignore = "differential JIT/interpreter hashes; run explicitly before changing native code"]
    fn jit_and_secure_jit_match_interpreter() {
        assert_ne!(flags() & JIT, 0, "this test requires a JIT backend");
        for seed in [b"jit differential seed A", b"jit differential seed B"] {
            // A light cache deliberately has no dataset-initialization JIT;
            // both VM JIT variants must remain available independently.
            let cache = Arc::new(
                Cache::with_tuning(
                    seed,
                    Tuning {
                        mode: MemoryMode::Light,
                        ..Tuning::default()
                    },
                )
                .unwrap(),
            );
            let make_vm = |extra| {
                // SAFETY: initialized shared cache; each light VM is owned by
                // its worker thread and keeps its own Arc to the cache.
                let ptr = unsafe {
                    randomx_create_vm(
                        flags() & HARD_AES | extra,
                        cache.0.as_ptr(),
                        std::ptr::null_mut(),
                    )
                };
                Vm {
                    ptr: NonNull::new(ptr).expect("allocate differential-test VM"),
                    huge_pages: false,
                    _dataset: None,
                    _cache: Some(cache.clone()),
                    _thread_local: PhantomData,
                    pending: false,
                }
            };
            // Generate and execute programs concurrently against one immutable cache.
            // VMs never cross thread boundaries; instruction-cache synchronization
            // must also remain correct when the OS migrates a worker between CPUs.
            let hashes = thread::scope(|scope| {
                let workers: Vec<_> = [0, JIT, JIT | SECURE]
                    .into_iter()
                    .map(|extra| {
                        let make_vm = &make_vm;
                        scope.spawn(move || {
                            let vm = make_vm(extra);
                            (0..16u32)
                                .map(|index| {
                                    let mut blob = [0u8; 76];
                                    for (position, byte) in blob.iter_mut().enumerate() {
                                        *byte = (index.wrapping_mul(131)
                                            ^ (position as u32).wrapping_mul(73))
                                            as u8;
                                    }
                                    blob[39..43].copy_from_slice(&index.to_le_bytes());
                                    let mut hash = [0; 32];
                                    // SAFETY: exclusive VM, valid input and 32-byte output.
                                    unsafe {
                                        randomx_calculate_hash(
                                            vm.ptr.as_ptr(),
                                            blob.as_ptr().cast(),
                                            blob.len(),
                                            hash.as_mut_ptr().cast(),
                                        );
                                    }
                                    thread::yield_now();
                                    hash
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .map(|worker| worker.join().unwrap())
                    .collect::<Vec<_>>()
            });
            assert_eq!(hashes[0], hashes[1], "JIT mismatch");
            assert_eq!(hashes[0], hashes[2], "secure JIT mismatch");
        }
    }

    #[test]
    fn ffi_pipeline_matches_consensus_vector_and_single_hashes() {
        let cache = Arc::new(Cache::new(b"test key 000").unwrap());
        let mut vm = Vm::create(None, Some(cache)).unwrap();
        let blobs: [&[u8]; 3] = [b"This is a test", b"second input", b"third input"];
        let expected: Vec<_> = blobs
            .iter()
            .map(|blob| {
                let mut hash = [0; 32];
                // SAFETY: exclusive initialized light VM and valid input/output buffers.
                unsafe {
                    randomx_calculate_hash(
                        vm.ptr.as_ptr(),
                        blob.as_ptr().cast(),
                        blob.len(),
                        hash.as_mut_ptr().cast(),
                    );
                }
                hash
            })
            .collect();
        assert_eq!(
            hex::encode(expected[0]),
            "639183aae1bf4c9a35884cb46b09cad9175f04efd7684e7262a0ac1c2f0b4e3f"
        );
        vm.first(blobs[0]);
        assert_eq!(vm.next(blobs[1]), expected[0]);
        assert_eq!(vm.next(blobs[2]), expected[1]);
        assert_eq!(vm.last(), expected[2]);
    }
}
