use crate::{
    control::Control,
    counters::{Counters, Writer},
    job::{Job, MAX_BLOB, NONCE_OFFSET},
    randomx::{Dataset, Tuning, Vm},
    topology::{self, Core},
};
use anyhow::{ensure, Context, Result};
use arc_swap::{cache::Cache, ArcSwapOption};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub type ShareFn = Arc<dyn Fn(&Job, u32, [u8; 32]) + Send + Sync>;

#[derive(Default)]
pub struct EngineStats {
    hashes: Arc<Counters>,
    pub threads: AtomicUsize,
    pub nodes: AtomicUsize,
    pub huge_datasets: AtomicUsize,
    pub giga_datasets: AtomicUsize,
    pub huge_workers: AtomicUsize,
    pub checksum: Arc<AtomicU64>,
}
impl EngineStats {
    pub fn hashes(&self) -> u64 {
        self.hashes.total()
    }
}

struct Node {
    node: Option<usize>,
    cores: Vec<Core>,
    dataset: Arc<Dataset>,
}

pub struct Engine {
    cores: Vec<Core>,
    init_cores: Vec<Core>,
    numa: bool,
    huge_pages: bool,
    tuning: Tuning,
    priority: Option<u8>,
    yield_cpu: bool,
    init_threads: Option<usize>,
    nonce_limit: u64,
    seed: Option<[u8; 32]>,
    nodes: Vec<Node>,
    workers: Vec<JoinHandle<()>>,
    running: Arc<AtomicBool>,
    jobs: Arc<ArcSwapOption<Job>>,
    control: Control,
    submit: ShareFn,
    pub stats: Arc<EngineStats>,
}

impl Engine {
    pub fn new(
        cores: Vec<Core>,
        init_cores: Vec<Core>,
        numa: bool,
        control: Control,
        submit: ShareFn,
    ) -> Self {
        Self {
            cores,
            init_cores,
            numa,
            huge_pages: false,
            tuning: Tuning::default(),
            priority: None,
            yield_cpu: true,
            init_threads: None,
            nonce_limit: 1u64 << 32,
            seed: None,
            nodes: Vec::new(),
            workers: Vec::new(),
            running: Arc::new(AtomicBool::new(false)),
            jobs: Arc::new(ArcSwapOption::empty()),
            control,
            submit,
            stats: Arc::new(EngineStats::default()),
        }
    }

    pub fn with_tuning(mut self, tuning: Tuning, priority: Option<u8>, yield_cpu: bool) -> Self {
        self.tuning = tuning;
        self.priority = priority;
        self.yield_cpu = yield_cpu;
        self
    }

    pub fn with_huge_pages(mut self, enabled: bool) -> Self {
        self.huge_pages = enabled;
        self
    }

    pub fn with_init_threads(mut self, threads: Option<usize>) -> Self {
        self.init_threads = threads;
        self
    }

    pub fn with_nonce_limit(mut self, limit: u64) -> Self {
        assert!((1..=1u64 << 32).contains(&limit));
        self.nonce_limit = limit;
        self
    }

    pub fn clear_job(&self) {
        self.jobs.store(None);
    }

    pub fn install(&mut self, job: Arc<Job>) -> Result<()> {
        ensure!(!self.control.stopped(), "engine is stopping");
        ensure!(!self.cores.is_empty(), "no worker CPUs selected");
        ensure!(
            (NONCE_OFFSET + 4..=MAX_BLOB).contains(&job.blob.len()),
            "invalid job blob length"
        );
        if self.seed != Some(job.seed) {
            self.stop();
            let started = Instant::now();
            self.nodes = match self.build_datasets(&job.seed) {
                Ok(nodes) => nodes,
                Err(error) if self.numa && !self.control.stopped() => {
                    crate::status!(
                        "[rx] NUMA initialization failed: {error:#}; falling back to one dataset"
                    );
                    self.numa = false;
                    self.build_datasets(&job.seed)?
                }
                Err(error) => return Err(error),
            };
            if let Err(error) = self.start_workers() {
                self.stop();
                return Err(error);
            }
            // Commit only after every worker has successfully created its VM.
            self.seed = Some(job.seed);
            self.stats.nodes.store(self.nodes.len(), Ordering::Relaxed);
            self.stats.huge_datasets.store(
                self.nodes
                    .iter()
                    .filter(|n| n.dataset.uses_huge_pages())
                    .count(),
                Ordering::Relaxed,
            );
            self.stats.giga_datasets.store(
                self.nodes
                    .iter()
                    .filter(|n| n.dataset.uses_giga_pages())
                    .count(),
                Ordering::Relaxed,
            );
            if self.tuning.one_gb_pages {
                crate::status!(
                    "[rx] explicit 1 GiB pages: datasets {}/{}",
                    self.stats.giga_datasets.load(Ordering::Relaxed),
                    self.nodes.len()
                );
            }
            if self.huge_pages {
                crate::status!("[rx] explicit 2 MiB pages: datasets {}/{}, scratchpads {}/{}; remaining allocations use ordinary/THP backing",
                    self.stats.huge_datasets.load(Ordering::Relaxed), self.nodes.len(),
                    self.stats.huge_workers.load(Ordering::Relaxed), self.cores.len());
            }
            if self.nodes.iter().any(|node| node.dataset.is_light()) {
                crate::status!("[rx] light mode: cache-backed workers");
            }
            crate::status!(
                "[rx] {} threads, {} dataset(s) ready ({} ms)",
                self.cores.len(),
                self.nodes.len(),
                started.elapsed().as_millis()
            );
        }
        self.jobs.store(Some(job));
        Ok(())
    }

    fn build_datasets(&self, seed: &[u8; 32]) -> Result<Vec<Node>> {
        let mut groups: BTreeMap<Option<usize>, Vec<Core>> = BTreeMap::new();
        for core in &self.cores {
            groups
                .entry(self.numa.then_some(core.node))
                .or_default()
                .push(core.clone());
        }
        thread::scope(|scope| {
            let mut handles = Vec::new();
            for (node, cores) in groups {
                let mut init: Vec<_> = self
                    .init_cores
                    .iter()
                    .filter(|c| node.is_none_or(|n| c.node == n))
                    .cloned()
                    .collect();
                if init.is_empty() {
                    init = cores.clone();
                }
                if let Some(limit) = self.init_threads {
                    init.truncate(limit.max(1));
                }
                let stop = &self.control.stop;
                let huge_pages = self.huge_pages;
                let tuning = self.tuning;
                handles.push(
                    thread::Builder::new()
                        .name(format!("rx-dataset-{node:?}"))
                        .spawn_scoped(scope, move || {
                            Dataset::build_with_tuning(seed, &init, node, stop, huge_pages, tuning)
                                .map(|dataset| Node {
                                    node,
                                    cores,
                                    dataset: Arc::new(dataset),
                                })
                        })
                        .context("spawn dataset allocator")?,
                );
            }
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .map_err(|_| anyhow::anyhow!("dataset allocator panicked"))?
                })
                .collect()
        })
    }

    fn start_workers(&mut self) -> Result<()> {
        let result = self.try_start_workers();
        if result.is_err() {
            self.stop_workers();
        }
        result
    }

    fn try_start_workers(&mut self) -> Result<()> {
        ensure!(!self.control.stopped(), "initialization interrupted");
        self.running.store(true, Ordering::Release);
        let mut writers = self.stats.hashes.writers(self.cores.len()).into_iter();
        let (ready_tx, ready_rx) = mpsc::channel();
        let mut index = 0;
        for node in &self.nodes {
            for core in &node.cores {
                let worker = Worker {
                    cpu: core.pinned.then_some(core.cpu),
                    checksum: self.stats.checksum.clone(),
                    nonce_limit: self.nonce_limit,
                    priority: self.priority,
                    yield_cpu: self.yield_cpu,
                    node: node.node,
                    dataset: node.dataset.clone(),
                    index,
                    count: self.cores.len(),
                    running: self.running.clone(),
                    jobs: self.jobs.clone(),
                    control: self.control.clone(),
                    counter: writers.next().expect("one counter per worker"),
                    submit: self.submit.clone(),
                };
                let ready = ready_tx.clone();
                self.workers.push(
                    thread::Builder::new()
                        .name(format!("rx-worker-{}", core.cpu))
                        .spawn(move || {
                            let control = worker.control.clone();
                            let _panic = control.watch_thread("RandomX worker");
                            if let Err(error) = worker.run(ready) {
                                control.fail(format!("RandomX worker failed: {error:#}"));
                            }
                        })
                        .context("spawn mining worker")?,
                );
                index += 1;
            }
        }
        drop(ready_tx);
        let mut huge_workers = 0;
        for _ in 0..self.cores.len() {
            huge_workers += usize::from(
                ready_rx
                    .recv()
                    .context("worker failed during VM initialization")?,
            );
        }
        ensure!(!self.control.stopped(), "worker initialization interrupted");
        self.stats
            .huge_workers
            .store(huge_workers, Ordering::Relaxed);
        self.stats
            .threads
            .store(self.workers.len(), Ordering::Relaxed);
        Ok(())
    }

    fn stop_workers(&mut self) {
        self.running.store(false, Ordering::Release);
        for worker in self.workers.drain(..) {
            if worker.join().is_err() {
                self.control.fail("RandomX worker panicked");
            }
        }
        self.jobs.store(None);
        self.stats.threads.store(0, Ordering::Relaxed);
        self.stats.huge_workers.store(0, Ordering::Relaxed);
    }

    pub fn stop(&mut self) {
        self.stop_workers();
        self.nodes.clear();
        self.seed = None;
        self.stats.nodes.store(0, Ordering::Relaxed);
        self.stats.huge_datasets.store(0, Ordering::Relaxed);
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Worker {
    checksum: Arc<AtomicU64>,
    cpu: Option<usize>,
    nonce_limit: u64,
    priority: Option<u8>,
    yield_cpu: bool,
    node: Option<usize>,
    dataset: Arc<Dataset>,
    index: usize,
    count: usize,
    running: Arc<AtomicBool>,
    jobs: Arc<ArcSwapOption<Job>>,
    control: Control,
    counter: Writer,
    submit: ShareFn,
}

impl Worker {
    fn report(&self, job: &Job, nonce: u32, hash: [u8; 32]) {
        if job.target == 0 {
            self.checksum.fetch_xor(
                u64::from_le_bytes(hash[..8].try_into().unwrap()),
                Ordering::Relaxed,
            );
        }
        if job.target != 0 && u64::from_le_bytes(hash[24..].try_into().unwrap()) < job.target {
            (self.submit)(job, nonce, hash);
        }
    }

    fn run(mut self, ready: mpsc::Sender<bool>) -> Result<()> {
        if let Some(cpu) = self.cpu {
            topology::pin(cpu).context("pin worker")?;
        }
        if let Some(node) = self.node {
            let _ = topology::bind_node(node);
        }
        topology::set_priority(self.priority, self.yield_cpu).context("set worker priority")?;
        crate::randomx::set_allocator_node(self.node);
        let mut vm = Vm::new(self.dataset.clone())?;
        ready
            .send(vm.uses_huge_pages())
            .context("worker readiness receiver closed")?;
        drop(ready);
        let mut jobs = Cache::new(self.jobs.clone());
        let mut buffers = [[0u8; MAX_BLOB]; 2];
        let mut blob_size = 0;
        let mut nonce_limit = self.nonce_limit;
        let mut current = 0;
        let mut pending_job: Option<Arc<Job>> = None;
        // Preserve each stream's nonce position across fee windows. Returning
        // to an unchanged user job must not submit the same nonces again.
        let mut progress: [Option<(Arc<Job>, u64)>; 2] = [None, None];
        let mut pending_nonce = 0;
        let mut cursor = self.index as u64;
        let mut pending = false;
        while self.running.load(Ordering::Acquire) && !self.control.stopped() {
            let job = jobs.load();
            let changed = match (job.as_ref(), pending_job.as_ref()) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            };
            if changed {
                if pending {
                    let hash = vm.last();
                    self.counter.record();
                    self.report(pending_job.as_ref().unwrap(), pending_nonce, hash);
                    pending = false;
                    cursor += self.count as u64;
                }
                if let Some(previous) = &pending_job {
                    progress[usize::from(previous.dev)] = Some((previous.clone(), cursor));
                }
                pending_job = job.as_ref().cloned();
                cursor = self.index as u64;
                if let Some(job) = &pending_job {
                    if let Some((previous, next)) = &progress[usize::from(job.dev)] {
                        if Arc::ptr_eq(previous, job) || previous.same_work(job) {
                            cursor = *next;
                        }
                    }
                    nonce_limit = self.nonce_limit.min(job.nonce_limit());
                    if cursor >= nonce_limit {
                        continue;
                    }
                    blob_size = job.blob.len();
                    buffers[0][..blob_size].copy_from_slice(&job.blob);
                    buffers[1][..blob_size].copy_from_slice(&job.blob);
                    current = 0;
                    pending_nonce = job.nonce_at(cursor);
                    buffers[0][NONCE_OFFSET..NONCE_OFFSET + 4]
                        .copy_from_slice(&pending_nonce.to_le_bytes());
                    vm.first(&buffers[0][..blob_size]);
                    pending = true;
                }
            }
            if !pending {
                self.counter.flush();
                thread::sleep(Duration::from_millis(1));
                continue;
            }
            let job = pending_job.as_ref().unwrap();
            for _ in 0..8 {
                if !self.running.load(Ordering::Relaxed) || self.control.stopped() {
                    break;
                }
                cursor += self.count as u64;
                if cursor >= nonce_limit {
                    let hash = vm.last();
                    self.counter.record();
                    self.report(job, pending_nonce, hash);
                    pending = false;
                    break;
                }
                let next_nonce = job.nonce_at(cursor);
                let next = 1 - current;
                buffers[next][NONCE_OFFSET..NONCE_OFFSET + 4]
                    .copy_from_slice(&next_nonce.to_le_bytes());
                let hash = vm.next(&buffers[next][..blob_size]);
                self.counter.record();
                self.report(job, pending_nonce, hash);
                current = next;
                pending_nonce = next_nonce;
            }
            self.counter.flush();
        }
        if pending {
            let hash = vm.last();
            self.counter.record();
            self.report(pending_job.as_ref().unwrap(), pending_nonce, hash);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::{HashMap, HashSet},
        sync::{atomic::AtomicU64, Mutex},
    };

    fn test_cores() -> (Vec<Core>, Vec<Core>) {
        let all = topology::discover().unwrap();
        let init = topology::select(&all, topology::cpu_quota().unwrap_or(8).min(8), false);
        let workers = init.iter().take(2).cloned().collect();
        (workers, init)
    }

    #[test]
    #[ignore = "allocates a full dataset; run explicitly with 4 GiB free"]
    fn full_dataset_shares_match_light_vm_across_fee_switches() {
        check_full_dataset_shares(false);
    }

    #[test]
    #[ignore = "allocates full datasets; uses reserved huge pages when available"]
    fn huge_dataset_shares_match_light_vm_across_fee_switches() {
        check_full_dataset_shares(true);
    }

    fn check_full_dataset_shares(huge_pages: bool) {
        let (cores, init) = test_cores();
        let control = Control::new(Arc::new(AtomicBool::new(false)));
        let results = Arc::new(Mutex::new(Vec::new()));
        let output = results.clone();
        let submit: ShareFn = Arc::new(move |job, nonce, hash| {
            output.lock().unwrap().push((job.clone(), nonce, hash));
        });
        let mut engine = Engine::new(cores.clone(), init, false, control.clone(), submit)
            .with_huge_pages(huge_pages);
        let stats = engine.stats.clone();
        let (monitor_stop, monitor_rx) = mpsc::channel::<()>();
        let observer = thread::spawn(move || {
            let mut previous = 0;
            while monitor_rx
                .recv_timeout(Duration::from_micros(100))
                .is_err_and(|e| e == mpsc::RecvTimeoutError::Timeout)
            {
                let count = stats.hashes();
                assert!(count >= previous, "hash count decreased during rebuild");
                assert!(stats.threads.load(Ordering::Relaxed) <= cores.len());
                assert!(stats.nodes.load(Ordering::Relaxed) <= 1);
                previous = count;
            }
        });
        let mut user = Job::benchmark([0; 32]);
        user.target = u64::MAX;
        let mut dev = user.clone();
        dev.dev = true;
        dev.blob[1] ^= 1;
        let user = Arc::new(user);
        let dev = Arc::new(dev);
        let count = |job: &Job| {
            results
                .lock()
                .unwrap()
                .iter()
                .filter(|(j, _, _)| j.same_work(job))
                .count()
        };
        let wait = |job: &Job, minimum: usize| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while count(job) < minimum {
                assert!(!control.stopped(), "{:?}", control.error());
                assert!(Instant::now() < deadline, "workers did not produce shares");
                thread::sleep(Duration::from_millis(5));
            }
        };
        engine.install(user.clone()).unwrap();
        if huge_pages && std::env::var_os("RXMINER_TEST_REQUIRE_HUGE").is_some() {
            assert_eq!(engine.stats.huge_datasets.load(Ordering::Relaxed), 1);
            assert_eq!(engine.stats.huge_workers.load(Ordering::Relaxed), 2);
        }
        wait(&user, 16);
        engine.install(dev.clone()).unwrap();
        wait(&dev, 16);
        let before = count(&user);
        engine.install(user.clone()).unwrap();
        wait(&user, before + 16);
        // New Arc and different height, but identical nonce work. This used
        // to restart every worker's nonce cursor and produce duplicate shares.
        let before = count(&user);
        let mut repeated = (*user).clone();
        repeated.height += 1;
        engine.install(Arc::new(repeated)).unwrap();
        wait(&user, before + 16);
        // Exercise rapid publication while workers finish outstanding hashes.
        for i in 0..32 {
            engine
                .install(if i % 2 == 0 {
                    dev.clone()
                } else {
                    user.clone()
                })
                .unwrap();
            thread::sleep(Duration::from_millis(2));
        }
        let mut next_seed = (*user).clone();
        next_seed.seed[0] = 1;
        let next_seed = Arc::new(next_seed);
        engine.install(next_seed.clone()).unwrap();
        if huge_pages && std::env::var_os("RXMINER_TEST_REQUIRE_HUGE").is_some() {
            // The old dataset and VMs must release their huge pages before
            // the new seed allocates from the same limited reservation.
            assert_eq!(engine.stats.huge_datasets.load(Ordering::Relaxed), 1);
            assert_eq!(engine.stats.huge_workers.load(Ordering::Relaxed), 2);
        }
        wait(&next_seed, 16);
        engine.stop();
        drop(monitor_stop);
        observer.join().unwrap();
        let results = results.lock().unwrap();
        assert_eq!(engine.stats.hashes(), results.len() as u64);
        assert_eq!(engine.stats.threads.load(Ordering::Relaxed), 0);
        assert_eq!(engine.stats.nodes.load(Ordering::Relaxed), 0);
        assert_eq!(engine.stats.huge_datasets.load(Ordering::Relaxed), 0);
        assert_eq!(engine.stats.huge_workers.load(Ordering::Relaxed), 0);
        let mut nonces = HashSet::new();
        let mut chosen: HashMap<([u8; 32], bool), Vec<_>> = HashMap::new();
        for (job, nonce, hash) in results.iter() {
            assert!(
                nonces.insert((job.seed, job.dev, job.id.clone(), *nonce)),
                "duplicate nonce after task update/fee switch"
            );
            let samples = chosen.entry((job.seed, job.dev)).or_default();
            if samples.len() < 4 {
                samples.push((job, nonce, hash));
            }
        }
        for ((seed, _), samples) in chosen {
            let blobs: Vec<_> = samples
                .iter()
                .map(|(job, nonce, _)| {
                    let mut blob = job.blob.clone();
                    blob[NONCE_OFFSET..NONCE_OFFSET + 4].copy_from_slice(&nonce.to_le_bytes());
                    blob
                })
                .collect();
            let expected = crate::randomx::light_hashes(&seed, &blobs);
            for ((_, _, actual), expected) in samples.into_iter().zip(expected) {
                assert_eq!(*actual, expected);
            }
        }
    }

    #[test]
    fn light_benchmark_checksum_is_independent_of_thread_count() {
        let job = Job::benchmark([0; 32]);
        let limit = 17;
        let blobs: Vec<_> = (0..limit)
            .map(|nonce: u32| {
                let mut blob = job.blob.clone();
                blob[NONCE_OFFSET..NONCE_OFFSET + 4].copy_from_slice(&nonce.to_le_bytes());
                blob
            })
            .collect();
        let expected = crate::randomx::light_hashes(&job.seed, &blobs)
            .iter()
            .fold(0u64, |value, hash| {
                value ^ u64::from_le_bytes(hash[..8].try_into().unwrap())
            });
        for count in [1, 2] {
            let (mut cores, init) = test_cores();
            cores.truncate(count);
            let control = Control::new(Arc::new(AtomicBool::new(false)));
            let mut engine = Engine::new(
                cores,
                init,
                false,
                control.clone(),
                Arc::new(|_, _, _| panic!("benchmark must not submit shares")),
            )
            .with_tuning(
                Tuning {
                    mode: crate::randomx::MemoryMode::Light,
                    ..Tuning::default()
                },
                None,
                true,
            )
            .with_nonce_limit(limit.into());
            engine.install(Arc::new(job.clone())).unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            while engine.stats.hashes() < limit.into()
                && Instant::now() < deadline
                && !control.stopped()
            {
                thread::sleep(Duration::from_millis(10));
            }
            engine.stop();
            assert!(control.error().is_none(), "{:?}", control.error());
            assert_eq!(engine.stats.hashes(), u64::from(limit));
            assert_eq!(engine.stats.checksum.load(Ordering::Relaxed), expected);
        }
    }

    #[test]
    #[ignore = "allocates full datasets; run explicitly with 4 GiB free"]
    fn counted_benchmark_finishes_exactly_with_mixed_affinity() {
        for limit in [1, 17] {
            let (mut cores, init) = test_cores();
            cores[0].pinned = false;
            let control = Control::new(Arc::new(AtomicBool::new(false)));
            let results = Arc::new(Mutex::new(Vec::new()));
            let received = results.clone();
            let mut engine = Engine::new(
                cores,
                init,
                false,
                control.clone(),
                Arc::new(move |_, nonce, hash| received.lock().unwrap().push((nonce, hash))),
            )
            .with_init_threads(Some(1))
            .with_nonce_limit(limit);
            let mut job = Job::benchmark([0; 32]);
            job.target = u64::MAX;
            engine.install(Arc::new(job.clone())).unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while engine.stats.hashes() < limit && Instant::now() < deadline && !control.stopped() {
                thread::sleep(Duration::from_millis(10));
            }
            thread::sleep(Duration::from_millis(50));
            engine.stop();
            assert!(control.error().is_none(), "{:?}", control.error());
            assert_eq!(engine.stats.hashes(), limit);
            let mut results = results.lock().unwrap().clone();
            results.sort_by_key(|(nonce, _)| *nonce);
            assert_eq!(
                results
                    .iter()
                    .map(|(nonce, _)| u64::from(*nonce))
                    .collect::<Vec<_>>(),
                (0..limit).collect::<Vec<_>>()
            );
            let blobs: Vec<_> = results
                .iter()
                .map(|(nonce, _)| {
                    let mut blob = job.blob.clone();
                    blob[NONCE_OFFSET..NONCE_OFFSET + 4].copy_from_slice(&nonce.to_le_bytes());
                    blob
                })
                .collect();
            assert_eq!(
                results
                    .into_iter()
                    .map(|(_, hash)| hash)
                    .collect::<Vec<_>>(),
                crate::randomx::light_hashes(&job.seed, &blobs)
            );
        }
    }

    #[test]
    #[ignore = "allocates full datasets; run explicitly with 4 GiB free"]
    fn worker_failures_stop_all_workers_and_clear_state() {
        let (cores, init) = test_cores();
        let mut invalid = cores.clone();
        invalid.last_mut().unwrap().cpu = 1 << 20;
        let control = Control::new(Arc::new(AtomicBool::new(false)));
        let mut engine = Engine::new(
            invalid,
            init.clone(),
            false,
            control.clone(),
            Arc::new(|_, _, _| {}),
        );
        assert!(engine.install(Arc::new(Job::benchmark([0; 32]))).is_err());
        assert!(engine.workers.is_empty() && engine.nodes.is_empty() && engine.seed.is_none());
        assert!(!engine.running.load(Ordering::Acquire));
        assert!(control.stopped() && control.error().is_some());
        assert_eq!(engine.stats.threads.load(Ordering::Relaxed), 0);
        assert_eq!(engine.stats.nodes.load(Ordering::Relaxed), 0);
        assert!(engine.install(Arc::new(Job::benchmark([0; 32]))).is_err());

        let control = Control::new(Arc::new(AtomicBool::new(false)));
        let callbacks = Arc::new(AtomicU64::new(0));
        let recorded = callbacks.clone();
        let mut engine = Engine::new(
            cores,
            init,
            false,
            control.clone(),
            Arc::new(move |_, _, _| {
                recorded.fetch_add(1, Ordering::Relaxed);
                panic!("injected submission panic");
            }),
        );
        let mut job = Job::benchmark([0; 32]);
        job.target = u64::MAX;
        engine.install(Arc::new(job)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !control.stopped() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(control.stopped(), "worker panic did not reach coordinator");
        engine.stop();
        assert!(engine.workers.is_empty() && engine.nodes.is_empty() && engine.seed.is_none());
        assert_eq!(engine.stats.hashes(), callbacks.load(Ordering::Relaxed));
        assert!(engine.stats.hashes() > 0);
        assert!(control.error().unwrap().contains("panicked"));
    }

    #[test]
    fn empty_or_stopped_engine_rejects_work() {
        let control = Control::new(Arc::new(AtomicBool::new(false)));
        let mut engine = Engine::new(
            vec![],
            vec![],
            false,
            control.clone(),
            Arc::new(|_, _, _| {}),
        );
        assert!(engine.install(Arc::new(Job::benchmark([0; 32]))).is_err());
        control.stop.store(true, Ordering::Release);
        assert!(engine.install(Arc::new(Job::benchmark([0; 32]))).is_err());
        assert_eq!(engine.stats.hashes(), 0);
    }
}
