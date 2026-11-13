//! Compile-time RandomX backend selection and miner-owned thread placement.
#[cfg(all(feature = "randomx-cpp", not(feature = "randomx-rust")))]
use randomx_cpp as backend;
#[cfg(feature = "randomx-rust")]
use randomx_rust as backend;

use crate::topology::{self, Core};
use anyhow::{ensure, Context, Result};
pub use backend::{
    commitment, configure_jit, set_allocator_node, set_memory_pool, set_thp, MemoryMode, Tuning,
    BACKEND, JIT_SUPPORTED,
};
use std::sync::{atomic::AtomicBool, Arc};

pub struct Dataset(Arc<backend::Dataset>);
impl Dataset {
    pub fn build(
        seed: &[u8; 32],
        cores: &[Core],
        node: Option<usize>,
        stop: &AtomicBool,
        huge_pages: bool,
    ) -> Result<Self> {
        Self::build_with_tuning(seed, cores, node, stop, huge_pages, Tuning::default())
    }
    pub fn build_with_tuning(
        seed: &[u8; 32],
        cores: &[Core],
        node: Option<usize>,
        stop: &AtomicBool,
        huge_pages: bool,
        tuning: Tuning,
    ) -> Result<Self> {
        ensure!(!cores.is_empty(), "dataset initialization requires CPUs");
        static NOTICE: std::sync::Once = std::sync::Once::new();
        NOTICE.call_once(|| {
            crate::status!("[rx] {} backend, runtime AES and page selection", BACKEND)
        });
        let dataset = backend::Dataset::build_with_tuning(
            seed,
            cores.len(),
            stop,
            huge_pages,
            tuning,
            |i| {
                if cores[i].pinned {
                    topology::pin(cores[i].cpu).context("pin RandomX initializer")?;
                }
                if let Some(node) = node {
                    let _ = topology::bind_node(node);
                }
                backend::set_allocator_node(node);
                Ok(())
            },
        )?;
        if tuning.mode == MemoryMode::Auto && dataset.is_light() {
            crate::status!("[rx] full-memory initialization unavailable; using light mode");
        }
        Ok(Self(Arc::new(dataset)))
    }
    pub fn is_light(&self) -> bool {
        self.0.is_light()
    }
    pub fn uses_huge_pages(&self) -> bool {
        self.0.uses_huge_pages()
    }
    pub fn uses_giga_pages(&self) -> bool {
        self.0.uses_giga_pages()
    }
    #[cfg(feature = "randomx-rust")]
    pub fn from_cache(
        cache: Arc<crate::randomx_core::cache::Cache>,
        huge_pages: bool,
        tuning: Tuning,
    ) -> Result<Self> {
        Ok(Self(Arc::new(backend::Dataset::from_cache(
            cache, huge_pages, tuning,
        )?)))
    }
    #[cfg(feature = "randomx-rust")]
    pub fn from_dataset(
        dataset: Arc<crate::randomx_core::vm::Dataset>,
        huge_pages: bool,
        tuning: Tuning,
    ) -> Result<Self> {
        Ok(Self(Arc::new(backend::Dataset::from_dataset(
            dataset, huge_pages, tuning,
        )?)))
    }
}

// Expose forwarding bodies for cross-crate inlining on the hash pipeline.
pub struct Vm(backend::Vm);
impl Vm {
    pub fn new(dataset: Arc<Dataset>) -> Result<Self> {
        Ok(Self(backend::Vm::new(dataset.0.clone())?))
    }
    pub fn set_dataset(&mut self, dataset: Arc<Dataset>) -> Result<()> {
        self.0.set_dataset(dataset.0.clone())
    }
    pub fn uses_huge_pages(&self) -> bool {
        self.0.uses_huge_pages()
    }
    #[inline]
    pub fn first(&mut self, blob: &[u8]) {
        self.0.first(blob);
    }
    #[inline]
    pub fn hash(&mut self, blob: &[u8], v2: bool) -> Result<[u8; 32]> {
        self.0.hash(blob, v2)
    }
    #[inline]
    pub fn next(&mut self, blob: &[u8]) -> Result<[u8; 32]> {
        self.0.next(blob)
    }
    #[inline]
    pub fn next_version(&mut self, blob: &[u8], v2: bool) -> Result<[u8; 32]> {
        self.0.next_version(blob, v2)
    }
    #[inline]
    pub fn last(&mut self) -> Result<[u8; 32]> {
        self.0.last()
    }
    #[inline]
    pub fn last_version(&mut self, v2: bool) -> Result<[u8; 32]> {
        self.0.last_version(v2)
    }
}

#[cfg(test)]
pub(crate) fn light_hashes(seed: &[u8], blobs: &[Vec<u8>]) -> Vec<[u8; 32]> {
    let tuning = Tuning {
        mode: MemoryMode::Light,
        jit: false,
        hw_aes: Some(false),
        ..Tuning::default()
    };
    let data = Arc::new(
        backend::Dataset::build_with_tuning(
            seed,
            1,
            &AtomicBool::new(false),
            false,
            tuning,
            |_| Ok(()),
        )
        .unwrap(),
    );
    let mut vm = backend::Vm::new(data).unwrap();
    blobs
        .iter()
        .map(|blob| vm.hash(blob, false).unwrap())
        .collect()
}
