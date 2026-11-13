// SPDX-License-Identifier: MIT
//! Miner adapter for the portable Rust RandomX implementation.
//! x86-64, ARM64 and Linux RV64 use Rust JITs; other targets use the portable core.
use crate::{
    randomx_core::{
        cache::Cache,
        vm::{self, DatasetSource},
    },
    randomx_memory::{self as memory, Policy},
};
use anyhow::{ensure, Result};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub const JIT_SUPPORTED: bool = cfg!(any(
    all(any(unix, windows), target_arch = "x86_64"),
    all(unix, target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "riscv64")
));

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
impl Default for Tuning {
    fn default() -> Self {
        Self {
            mode: MemoryMode::default(),
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
            !self.jit || JIT_SUPPORTED,
            "JIT is unavailable on this platform"
        );
        if self.hw_aes == Some(true) {
            #[cfg(target_arch = "x86_64")]
            ensure!(
                std::is_x86_feature_detected!("aes"),
                "hardware AES unavailable on this CPU"
            );
            #[cfg(target_arch = "aarch64")]
            ensure!(
                std::arch::is_aarch64_feature_detected!("aes"),
                "hardware AES unavailable on this CPU"
            );
            #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
            anyhow::bail!("hardware AES is unavailable on this platform");
        }
        #[cfg(any(
            all(any(unix, windows), target_arch = "x86_64"),
            all(unix, target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "riscv64")
        ))]
        ensure!(
            crate::randomx_jit::argon2::available(self.argon2),
            "requested Argon2 backend unavailable (scalar, SSSE3, AVX2 or auto)"
        );
        #[cfg(not(any(
            all(any(unix, windows), target_arch = "x86_64"),
            all(unix, target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "riscv64")
        )))]
        ensure!(
            self.argon2.is_none_or(|value| value == 0),
            "only scalar Argon2 is ported on this platform"
        );
        Ok(())
    }
}

pub fn set_memory_pool(pages: usize) {
    memory::set_pool(pages);
}
pub fn set_allocator_node(node: Option<usize>) {
    memory::set_node(node);
}
pub fn configure_jit(huge_pages: bool, prefetch_mode: u8) {
    assert!(prefetch_mode <= 3, "prefetch mode must be 0..3");
    memory::JIT_HUGE.store(huge_pages, Ordering::Relaxed);
    memory::PREFETCH.store(prefetch_mode, Ordering::Relaxed);
}
pub fn set_thp(enabled: bool) {
    memory::set_thp(enabled);
}

#[derive(Clone)]
enum Storage {
    Light(Arc<Cache>),
    Full(Arc<vm::Dataset>),
}
pub struct Dataset {
    storage: Storage,
    tuning: Tuning,
    huge_pages: bool,
}
impl DatasetSource for Dataset {
    fn item(&self, index: u64) -> [u64; 8] {
        match &self.storage {
            Storage::Light(cache) => {
                #[cfg(target_arch = "x86_64")]
                return crate::randomx_interpreter::dataset_item(cache, index);
                #[cfg(not(target_arch = "x86_64"))]
                cache.item(index)
            }
            Storage::Full(dataset) => dataset.item(index),
        }
    }
    fn prefetch(&self, index: u64) {
        #[cfg(target_arch = "x86_64")]
        if let Storage::Full(data) = &self.storage {
            let words = data.words();
            if let Some(offset) = index
                .checked_mul(8)
                .and_then(|n| usize::try_from(n).ok())
                .filter(|&n| n < words.len())
            {
                // SAFETY: checked element within the live immutable dataset.
                unsafe {
                    std::arch::x86_64::_mm_prefetch(
                        words.as_ptr().add(offset).cast(),
                        std::arch::x86_64::_MM_HINT_NTA,
                    );
                }
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        let _ = index;
    }
}
impl Dataset {
    /// Use an existing initialized cache without rebuilding it.
    pub fn from_cache(cache: Arc<Cache>, huge_pages: bool, tuning: Tuning) -> Result<Self> {
        tuning.validate()?;
        Ok(Self {
            storage: Storage::Light(cache),
            huge_pages,
            tuning,
        })
    }
    /// Share an existing full dataset between VMs with independent execution settings.
    pub fn from_dataset(
        dataset: Arc<vm::Dataset>,
        huge_pages: bool,
        tuning: Tuning,
    ) -> Result<Self> {
        tuning.validate()?;
        Ok(Self {
            storage: Storage::Full(dataset),
            huge_pages,
            tuning,
        })
    }
    pub fn is_light(&self) -> bool {
        matches!(self.storage, Storage::Light(_))
    }
    fn page_size(&self) -> usize {
        match &self.storage {
            Storage::Light(v) => v.page_size(),
            Storage::Full(v) => v.page_size(),
        }
    }
    pub fn uses_huge_pages(&self) -> bool {
        self.page_size() == 2 << 20
    }
    pub fn uses_giga_pages(&self) -> bool {
        self.page_size() == 1 << 30
    }
    /// Build immutable seed data. `setup` configures the allocating thread (0)
    /// and each initializer worker before it touches memory.
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
        #[cfg(any(
            all(any(unix, windows), target_arch = "x86_64"),
            all(unix, target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "riscv64")
        ))]
        let cache = crate::randomx_jit::new_cache_with_policy(
            seed,
            tuning.argon2,
            Policy {
                huge: huge_pages,
                giga: false,
            },
        )?;
        #[cfg(not(any(
            all(any(unix, windows), target_arch = "x86_64"),
            all(unix, target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "riscv64")
        )))]
        let cache = Cache::with_memory(
            seed,
            memory::Mapped::<[u64; 128]>::new(
                crate::randomx_core::cache::CACHE_SIZE / 1024,
                Policy {
                    huge: huge_pages,
                    giga: false,
                },
            )?
            .into_memory(),
            crate::randomx_core::cache::fill_memory,
        )?;
        ensure!(!stop.load(Ordering::Relaxed), "initialization interrupted");
        if tuning.mode != MemoryMode::Light {
            let setup = |i: usize| {
                setup(i).map_err(|error| {
                    crate::randomx_core::cache::CacheError::Thread(std::io::Error::other(
                        error.to_string(),
                    ))
                })
            };
            let policy = Policy {
                huge: huge_pages,
                giga: tuning.one_gb_pages,
            };
            let portable = || -> Result<vm::Dataset, crate::randomx_core::cache::CacheError> {
                let mut items = memory::Mapped::<[u64; 8]>::new(
                    crate::randomx_core::cache::DATASET_ITEMS,
                    policy,
                )
                .map_err(crate::randomx_core::cache::CacheError::Backend)?;
                vm::initialize_dataset(
                    &|start, output| cache.fill_dataset(start, output),
                    items.as_mut(),
                    threads,
                    stop,
                    &setup,
                )?;
                vm::Dataset::from_memory(items.into_memory())
            };
            #[cfg(any(
                all(any(unix, windows), target_arch = "x86_64"),
                all(unix, target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "riscv64")
            ))]
            let built = if tuning.jit {
                crate::randomx_jit::Initializer::new(&cache)
                    .map_err(crate::randomx_core::cache::CacheError::Backend)
                    .and_then(|initializer| {
                        initializer.build_dataset_with_policy(threads, stop, setup, policy)
                    })
            } else {
                portable()
            };
            #[cfg(not(any(
                all(any(unix, windows), target_arch = "x86_64"),
                all(unix, target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "riscv64")
            )))]
            let built = portable();
            match built {
                Ok(dataset) => {
                    return Ok(Self {
                        storage: Storage::Full(Arc::new(dataset)),
                        tuning,
                        huge_pages,
                    })
                }
                Err(_error) if tuning.mode == MemoryMode::Auto && !stop.load(Ordering::Relaxed) => {
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Self {
            storage: Storage::Light(Arc::new(cache)),
            tuning,
            huge_pages,
        })
    }
}

enum Engine {
    #[cfg(any(
        all(any(unix, windows), target_arch = "x86_64"),
        all(unix, target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "riscv64")
    ))]
    Jit(Box<crate::randomx_jit::Vm>),
    Interpreter(vm::Vm),
}
pub struct Vm {
    engine: Engine,
    dataset: Arc<Dataset>,
}
impl Vm {
    /// Create independent execution settings over an existing immutable seed.
    /// The dataset determines full/light memory mode; its cache is not rebuilt.
    pub fn with_tuning(dataset: Arc<Dataset>, tuning: Tuning) -> Result<Self> {
        tuning.validate()?;
        Self::new(Arc::new(Dataset {
            storage: dataset.storage.clone(),
            tuning,
            huge_pages: dataset.huge_pages,
        }))
    }

    pub fn uses_huge_pages(&self) -> bool {
        match &self.engine {
            #[cfg(any(
                all(any(unix, windows), target_arch = "x86_64"),
                all(unix, target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "riscv64")
            ))]
            Engine::Jit(v) => v.uses_huge_pages(),
            Engine::Interpreter(v) => v.page_size() == 2 << 20,
        }
    }
    pub fn new(dataset: Arc<Dataset>) -> Result<Self> {
        #[cfg(any(
            all(any(unix, windows), target_arch = "x86_64"),
            all(unix, target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "riscv64")
        ))]
        if dataset.tuning.jit {
            use crate::randomx_jit::{Source, Vm as JitVm, VmOptions};
            let source = match &dataset.storage {
                Storage::Light(cache) => Source::Light(cache.clone()),
                Storage::Full(data) => Source::Full(data.clone()),
            };
            let vm = JitVm::with_options(
                source,
                VmOptions {
                    hardware_aes: dataset.tuning.hw_aes != Some(false),
                    huge_pages: dataset.huge_pages,
                    secure: dataset.tuning.secure,
                    ..VmOptions::default()
                },
            )?;
            return Ok(Self {
                engine: Engine::Jit(Box::new(vm)),
                dataset,
            });
        }
        #[cfg(target_arch = "aarch64")]
        use crate::randomx_aes_arm::HardwareAes;
        #[cfg(not(target_arch = "x86_64"))]
        use crate::randomx_core::aes::Aes as SoftwareAes;
        #[cfg(target_arch = "x86_64")]
        use crate::randomx_jit::aes::HardwareAes;
        #[cfg(target_arch = "x86_64")]
        use crate::randomx_jit::aes::SoftwareAes;
        let aes: Box<dyn crate::randomx_core::aes::Backend> = {
            #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
            if dataset.tuning.hw_aes != Some(false) {
                if let Some(aes) = HardwareAes::new() {
                    Box::new(aes)
                } else {
                    Box::new(SoftwareAes::default())
                }
            } else {
                Box::new(SoftwareAes::default())
            }
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            {
                Box::new(SoftwareAes::default())
            }
        };
        let scratch = crate::randomx_core::memory::Memory::Platform(Box::new(
            memory::Scratchpad::new(dataset.huge_pages)?,
        ));
        let vm = vm::Vm::with_backends(scratch, aes)?;
        #[cfg(target_arch = "x86_64")]
        let vm = vm.with_executor(Box::new(crate::randomx_interpreter::Interpreter::new(
            dataset.tuning.hw_aes != Some(false),
        )));
        Ok(Self {
            engine: Engine::Interpreter(vm),
            dataset,
        })
    }
    /// Replace the seed data only between completed hash pipelines.
    pub fn set_dataset(&mut self, dataset: Arc<Dataset>) -> Result<()> {
        ensure!(
            self.dataset.is_light() == dataset.is_light(),
            "replacement must preserve full/light mode"
        );
        match &mut self.engine {
            #[cfg(any(
                all(any(unix, windows), target_arch = "x86_64"),
                all(unix, target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "riscv64")
            ))]
            Engine::Jit(vm) => {
                use crate::randomx_jit::Source;
                vm.set_source(match &dataset.storage {
                    Storage::Light(v) => Source::Light(v.clone()),
                    Storage::Full(v) => Source::Full(v.clone()),
                })?;
            }
            Engine::Interpreter(vm) => ensure!(
                !vm.has_pending(),
                "cannot replace data while a hash is pending"
            ),
        }
        self.dataset = dataset;
        Ok(())
    }
    pub fn first(&mut self, blob: &[u8]) {
        match &mut self.engine {
            #[cfg(any(
                all(any(unix, windows), target_arch = "x86_64"),
                all(unix, target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "riscv64")
            ))]
            Engine::Jit(v) => v.first(blob),
            Engine::Interpreter(v) => v.first(blob),
        }
    }
    pub fn hash(&mut self, blob: &[u8], v2: bool) -> Result<[u8; 32]> {
        self.first(blob);
        self.last_version(v2)
    }
    pub fn next(&mut self, blob: &[u8]) -> Result<[u8; 32]> {
        self.next_version(blob, false)
    }
    pub fn next_version(&mut self, blob: &[u8], v2: bool) -> Result<[u8; 32]> {
        Ok(match &mut self.engine {
            #[cfg(any(
                all(any(unix, windows), target_arch = "x86_64"),
                all(unix, target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "riscv64")
            ))]
            Engine::Jit(v) => v.next(blob, v2)?,
            Engine::Interpreter(v) => v.next(&*self.dataset, blob, v2),
        })
    }
    pub fn last(&mut self) -> Result<[u8; 32]> {
        self.last_version(false)
    }
    pub fn last_version(&mut self, v2: bool) -> Result<[u8; 32]> {
        Ok(match &mut self.engine {
            #[cfg(any(
                all(any(unix, windows), target_arch = "x86_64"),
                all(unix, target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "riscv64")
            ))]
            Engine::Jit(v) => v.last(v2)?,
            Engine::Interpreter(v) => v.last(&*self.dataset, v2),
        })
    }
}
