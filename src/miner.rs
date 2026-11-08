use crate::{
    config::{Benchmark, Options},
    control::Control,
    engine::{Engine, EngineStats, ShareFn},
    http,
    job::Job,
    stratum::{Client, ClientState},
    topology,
};
use anyhow::{bail, ensure, Result};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const DEV_ADDRESS: &str = "46fca3jxDH5RxoRkAg2DcCPYWyMGFLYQgRZrYTR7qyd2GLqLq8hvSiMSRQ5YWcZ5QBfYB1nCBb2J2aiww7bP7YHC21wZecJ";

struct Status {
    started: Instant,
    engine: Arc<EngineStats>,
    user: Arc<ClientState>,
    dev: Arc<ClientState>,
    dev_phase: AtomicBool,
    paused: AtomicBool,
    fee: bool,
    pool_url: String,
    samples: Mutex<VecDeque<(Instant, u64)>>,
}
impl Status {
    fn sample(&self) {
        let now = Instant::now();
        let mut samples = self.samples.lock().unwrap();
        samples.push_back((now, self.engine.hashes()));
        while samples
            .front()
            .is_some_and(|(time, _)| now.duration_since(*time) > Duration::from_secs(960))
        {
            samples.pop_front();
        }
    }
    fn rate(&self, seconds: u64) -> f64 {
        if self.paused.load(Ordering::Acquire) {
            return 0.0;
        }
        let samples = self.samples.lock().unwrap();
        let Some(&(end, hashes)) = samples.back() else {
            return 0.0;
        };
        let Some(&(start, first)) = samples
            .iter()
            .find(|(time, _)| end.duration_since(*time) <= Duration::from_secs(seconds))
        else {
            return 0.0;
        };
        let elapsed = end.duration_since(start).as_secs_f64();
        if elapsed < 0.1 {
            0.0
        } else {
            hashes.saturating_sub(first) as f64 / elapsed
        }
    }
    fn rates(&self) -> Value {
        json!({"10s":self.rate(10),"60s":self.rate(60),"15m":self.rate(900),
            "total":self.engine.hashes() as f64 / self.started.elapsed().as_secs_f64().max(0.001)})
    }
    fn response(&self, path: &str) -> (u16, Value) {
        let version = env!("CARGO_PKG_VERSION");
        (
            200,
            match path {
                "/" => {
                    json!({"name":"RxMiner","version":version,"algorithm":"rx/0","endpoints":["/summary","/hashrates","/shares"]})
                }
                "/hashrates" => self.rates(),
                "/shares" => json!({"user":self.user.shares(),"dev":self.dev.shares()}),
                "/summary" => json!({"name":"RxMiner","version":version,"algorithm":"rx/0",
                "uptime":self.started.elapsed().as_secs_f64(), "hashes":self.engine.hashes(), "paused":self.paused.load(Ordering::Acquire),
                "hashrate":self.rates(), "shares":self.user.shares(), "dev_shares":self.dev.shares(),
                "threads":self.engine.threads.load(Ordering::Relaxed), "nodes":self.engine.nodes.load(Ordering::Relaxed),
                "huge_pages":{"size_kib":2048,"datasets":self.engine.huge_datasets.load(Ordering::Relaxed),"giga_datasets":self.engine.giga_datasets.load(Ordering::Relaxed),"scratchpads":self.engine.huge_workers.load(Ordering::Relaxed)},
                "dev_fee":{"percent":if self.fee {"<1%"} else {"0%"}, "phase":if self.dev_phase.load(Ordering::Relaxed) {"dev"} else {"user"}, "address":DEV_ADDRESS},
                "pool":{"url":self.pool_url,"connected":self.user.connected.load(Ordering::Acquire)},
                "dev_pool":{"url":self.dev.endpoint.lock().unwrap_or_else(|e| e.into_inner()).clone(),
                    "connected":self.dev.connected.load(Ordering::Acquire)}}),
                _ => return (404, json!({"error":"not found","path":path})),
            },
        )
    }
    fn health(&self) {
        let available = topology::available_memory()
            .map(|bytes| bytes / (1024 * 1024))
            .map_or_else(|| "unknown".into(), |m| format!("{m} MiB"));
        crate::status!(
            "[health] workers {}, memory available {}",
            self.engine.threads.load(Ordering::Relaxed),
            available
        );
    }
    fn print(&self, final_line: bool) {
        crate::status!(
            "[speed] 10s/60s/15m {:.1} {:.1} {:.1} H/s | shares {}/{}/{}{}",
            self.rate(10),
            self.rate(60),
            self.rate(900),
            self.user.submitted.load(Ordering::Relaxed),
            self.user.accepted.load(Ordering::Relaxed),
            self.user.rejected.load(Ordering::Relaxed),
            if final_line { " [final]" } else { "" }
        );
    }
}

fn api_response(
    status: &Status,
    options: &Options,
    control: &Control,
    method: &str,
    path: &str,
    body: &[u8],
) -> (u16, Value) {
    if path == "/json_rpc" {
        if options.http_restricted {
            return (403, json!({"error":"API is restricted"}));
        }
        if method != "POST" {
            return (405, json!({"error":"POST required"}));
        }
        let request: Value = match serde_json::from_slice(body) {
            Ok(value) => value,
            Err(_) => {
                return (
                    400,
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
                )
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let command = request["method"].as_str().unwrap_or_default();
        match command {
            "pause" if options.benchmark.is_none() => control.paused.store(true, Ordering::Release),
            "resume" if options.benchmark.is_none() => {
                control.paused.store(false, Ordering::Release)
            }
            "stop" => control.stop.store(true, Ordering::Release),
            _ => {
                return (
                    200,
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not available"}}),
                )
            }
        }
        return (
            200,
            json!({"jsonrpc":"2.0","id":id,"result":{"status":"OK"}}),
        );
    }
    if path == "/1/config" || path == "/2/config" {
        if options.http_restricted {
            return (403, json!({"error":"API is restricted"}));
        }
        let Some(path) = &options.source_cli.config else {
            return (
                409,
                json!({"error":"start with --config to manage configuration"}),
            );
        };
        if !matches!(method, "GET" | "PUT") {
            return (405, json!({"error":"GET or PUT required"}));
        }
        let result: anyhow::Result<Value> = (|| {
            if method == "GET" {
                return Ok(serde_json::from_slice(&std::fs::read(path)?)?);
            }
            let replacement = Options::from_json(options.source_cli.clone(), body)?;
            ensure!(
                replacement.background == options.background,
                "changing background mode requires restarting the process"
            );
            replacement.select_cores(&topology::discover()?, topology::cpu_quota())?;
            crate::runtime::atomic_write(path, body)?;
            control.reload.store(true, Ordering::Release);
            Ok(json!({"status":"OK"}))
        })();
        return match result {
            Ok(value) => (200, value),
            Err(error) => (400, json!({"error":format!("{error:#}")})),
        };
    }
    if method != "GET" {
        return (405, json!({"error":"method not allowed"}));
    }
    if matches!(path, "/1/summary" | "/2/summary") {
        return (
            200,
            json!({"id":"rxminer","worker_id":options.pool.as_ref().map(|p| &p.rig_id),
            "version":env!("CARGO_PKG_VERSION"),"kind":"miner","ua":concat!("RxMiner/", env!("CARGO_PKG_VERSION")),
            "algo":"rx/0","uptime":status.started.elapsed().as_secs(),"paused":status.paused.load(Ordering::Acquire),
            "hashrate":{"total":[status.rate(10),status.rate(60),status.rate(900)]},
            "results":{"hashes_total":status.engine.hashes(),"shares_good":status.user.accepted.load(Ordering::Relaxed),
                "shares_total":status.user.submitted.load(Ordering::Relaxed)},
            "connection":{"pool":status.pool_url,"connected":status.user.connected.load(Ordering::Acquire)}}),
        );
    }
    status.response(path)
}

pub fn run(options: Options, stop: Arc<AtomicBool>) -> Result<()> {
    run_control(options, Control::new(stop))
}

pub(crate) fn run_control(options: Options, control: Control) -> Result<()> {
    let result = run_inner(options, control.clone());
    let interrupted = control.stopped();
    control.stop.store(true, Ordering::Relaxed);
    if let Some(error) = control.error() {
        bail!(error);
    }
    if interrupted {
        return Ok(());
    }
    result
}

fn run_inner(options: Options, control: Control) -> Result<()> {
    let started = Instant::now();
    let all = topology::discover()?;
    let quota = topology::cpu_quota();
    if let Some(quota) = quota {
        crate::status!("cgroup CPU quota: {quota} CPUs");
    }
    let cores = options.select_cores(&all, quota)?;
    let init_cores = topology::select(&all, quota.unwrap_or(0), false);
    let _msr = crate::msr::apply(
        &all.iter().map(|core| core.cpu).collect::<Vec<_>>(),
        &options.msr,
        options.restore_msr,
    )?;
    ensure!(!cores.is_empty(), "no usable CPU cores found");
    let numa = topology::use_numa(&cores, options.numa, topology::available_memory());
    crate::status!("RxMiner {} / rx/0", env!("CARGO_PKG_VERSION"));
    crate::status!("CPU: {}", topology::summary(&cores));
    crate::status!(
        "NUMA: {}, THP: {}",
        if numa {
            "on (one dataset per node)"
        } else {
            "off (single dataset)"
        },
        if options.thp { "on" } else { "off" }
    );
    let mut user = None;
    let mut dev = None;
    let mut fee = None;
    if let Some(pool) = &options.pool {
        crate::status!("pool: {}", pool.address.url);
        crate::status!("user: {}", pool.user);
        if !pool.rig_id.is_empty() {
            crate::status!("rig-id: {}", pool.rig_id);
        }
        crate::status!(
            "connection: {} | proxy: {}",
            if !pool.address.tls {
                "TCP"
            } else if !pool.tls.fingerprint.is_empty() && pool.tls.verify {
                "TLS (certificate and fingerprint verification)"
            } else if !pool.tls.fingerprint.is_empty() {
                "TLS (fingerprint verification)"
            } else if pool.tls.verify {
                "TLS (certificate verification)"
            } else {
                "TLS (unverified)"
            },
            pool.proxy
                .as_ref()
                .map_or_else(|| "none (direct)".into(), |proxy| proxy.label())
        );
        crate::status!("dev fee: <1%");
        fee = Some(FeeSchedule::new()?);
        let dev_pools = crate::developer::pools(&options)?;
        user = Some(Client::start(pool.clone(), false, control.clone())?);
        dev = Some(Client::start_developer(dev_pools, control.clone())?);
    }
    let user_state = user.as_ref().map(|c| c.state.clone()).unwrap_or_default();
    let dev_state = dev.as_ref().map(|c| c.state.clone()).unwrap_or_default();
    let submit: ShareFn = if let (Some(user), Some(dev)) = (&user, &dev) {
        let user = user.submitter.clone();
        let dev = dev.submitter.clone();
        Arc::new(move |job, nonce, hash| {
            if job.dev {
                dev.submit(job, nonce, hash);
            } else {
                user.submit(job, nonce, hash);
            }
        })
    } else {
        Arc::new(|_, _, _| {})
    };
    let mut tuning = options.tuning;
    if tuning.mode == crate::randomx::MemoryMode::Auto
        && topology::available_memory()
            .is_some_and(|bytes| bytes < (2304u64 << 20) + ((cores.len() as u64) << 21))
    {
        tuning.mode = crate::randomx::MemoryMode::Light;
        crate::status!("[rx] memory budget selects light mode");
    }
    crate::status!(
        "RandomX: mode {}, AES {}, huge pages {}",
        match tuning.mode {
            crate::randomx::MemoryMode::Auto => "auto",
            crate::randomx::MemoryMode::Fast => "fast (full memory)",
            crate::randomx::MemoryMode::Light => "light",
        },
        match tuning.hw_aes {
            Some(true) => "hardware",
            Some(false) => "software",
            None => "auto",
        },
        if tuning.one_gb_pages {
            "1 GiB requested"
        } else if options.huge_pages {
            "2 MiB requested"
        } else {
            "off"
        }
    );
    let mut engine = Engine::new(cores, init_cores, numa, control.clone(), submit)
        .with_tuning(tuning, options.priority, options.yield_cpu)
        .with_huge_pages(options.huge_pages)
        .with_init_threads(options.init_threads);
    if let Some(Benchmark::Hashes(limit)) = options.benchmark {
        engine = engine.with_nonce_limit(limit);
    }
    let status = Arc::new(Status {
        started,
        engine: engine.stats.clone(),
        user: user_state.clone(),
        dev: dev_state.clone(),
        dev_phase: AtomicBool::new(fee.is_some()),
        paused: AtomicBool::new(false),
        fee: fee.is_some(),
        pool_url: options
            .pool
            .as_ref()
            .map(|p| p.address.url.clone())
            .unwrap_or_default(),
        samples: Mutex::new(VecDeque::new()),
    });
    let _http = if options.http_port > 0 {
        let status = status.clone();
        let api_control = control.clone();
        let api_options = options.clone();
        Some(http::Server::start_tls(
            &options.http_host,
            options.http_port,
            options.http_access_token.clone(),
            options.server_tls.acceptor()?,
            Arc::new(move |method, path, body| {
                api_response(&status, &api_options, &api_control, method, path, body)
            }),
            control.clone(),
        )?)
    } else {
        None
    };
    if let Some(benchmark) = options.benchmark {
        engine.install(Arc::new(Job::benchmark(options.seed)))?;
        status.sample();
        let start = Instant::now();
        let mut last_print = start;
        let mut last_health = start;
        while !control.stopped()
            && match benchmark {
                Benchmark::Seconds(seconds) => start.elapsed() < Duration::from_secs(seconds),
                Benchmark::Hashes(hashes) => status.engine.hashes() < hashes,
            }
        {
            thread::sleep(Duration::from_millis(100));
            status.sample();
            if options.health_print_time > 0
                && last_health.elapsed() >= Duration::from_secs(options.health_print_time)
            {
                status.health();
                last_health = Instant::now();
            }
            if options.print_time > 0
                && last_print.elapsed() >= Duration::from_secs(options.print_time)
            {
                status.print(false);
                last_print = Instant::now();
            }
        }
        engine.stop();
        status.sample();
        let elapsed = start.elapsed().as_secs_f64();
        let hashes = status.engine.hashes();
        crate::status!(
            "benchmark finished: {hashes} hashes in {elapsed:.3}s = {:.1} H/s",
            hashes as f64 / elapsed
        );
        let checksum = status.engine.checksum.load(Ordering::Relaxed);
        crate::status!("benchmark checksum: {checksum:016X}");
        if let Some(expected) = options.expected_hash {
            if !control.stopped() {
                ensure!(
                    checksum == expected,
                    "benchmark checksum mismatch: expected {expected:016X}, got {checksum:016X}"
                );
                crate::status!("benchmark checksum verified");
            }
        }
    } else {
        let mut schedule = fee.unwrap();
        let mut seed_wait = None;
        let dev_job = loop {
            ensure!(!control.stopped(), "pool job unavailable");
            if let Some((_, dev)) =
                synchronized_jobs(&user_state, &dev_state, &mut seed_wait, Instant::now())?
            {
                break dev;
            }
            thread::sleep(Duration::from_millis(25));
        };
        engine.install(dev_job.clone())?;
        schedule.start(Instant::now());
        status.sample();
        let mut active = dev_job;
        let mut last_sample = Instant::now();
        let mut last_print = Instant::now();
        let mut last_health = Instant::now();
        let mut paused_at: Option<Instant> = None;
        let mut last_power_check = Instant::now();
        let mut battery = options.pause_on_battery && crate::power::on_battery();
        while !control.stopped() {
            let jobs = synchronized_jobs(&user_state, &dev_state, &mut seed_wait, Instant::now())?;
            if options.pause_on_battery && last_power_check.elapsed() >= Duration::from_secs(1) {
                battery = crate::power::on_battery();
                last_power_check = Instant::now();
            }
            if battery || control.paused.load(Ordering::Acquire) || jobs.is_none() {
                if paused_at.is_none() {
                    engine.clear_job();
                    status.paused.store(true, Ordering::Release);
                    paused_at = Some(Instant::now());
                }
                if last_sample.elapsed() >= Duration::from_millis(500) {
                    status.sample();
                    last_sample = Instant::now();
                }
                thread::sleep(Duration::from_millis(25));
                continue;
            }
            let resumed = if let Some(paused) = paused_at.take() {
                schedule.deadline += paused.elapsed();
                status.paused.store(false, Ordering::Release);
                true
            } else {
                false
            };
            let phase = schedule.advance(Instant::now());
            status.dev_phase.store(phase, Ordering::Relaxed);
            let (user_job, dev_job) = jobs.unwrap();
            let next = if phase { dev_job } else { user_job };
            if resumed || !Arc::ptr_eq(&next, &active) {
                let rebuilding = next.seed != active.seed;
                let started = Instant::now();
                engine.install(next.clone())?;
                if rebuilding {
                    schedule.deadline += started.elapsed();
                }
                active = next;
            }
            if last_sample.elapsed() >= Duration::from_millis(500) {
                status.sample();
                last_sample = Instant::now();
            }
            if options.health_print_time > 0
                && last_health.elapsed() >= Duration::from_secs(options.health_print_time)
            {
                status.health();
                last_health = Instant::now();
            }
            if options.print_time > 0
                && last_print.elapsed() >= Duration::from_secs(options.print_time)
            {
                status.print(false);
                last_print = Instant::now();
            }
            thread::sleep(Duration::from_millis(5));
        }
        engine.stop();
        status.sample();
    }
    status.print(true);
    Ok(())
}

fn synchronized_jobs(
    user: &ClientState,
    dev: &ClientState,
    seed_wait: &mut Option<Instant>,
    now: Instant,
) -> Result<Option<(Arc<Job>, Arc<Job>)>> {
    if user.connected.load(Ordering::Acquire) && dev.connected.load(Ordering::Acquire) {
        if let (Some(user), Some(dev)) = (user.job.load_full(), dev.job.load_full()) {
            if user.seed == dev.seed {
                *seed_wait = None;
                return Ok(Some((user, dev)));
            }
            // Independent pools can publish an epoch change at different times.
            // Pause both work and the fee clock until their seeds converge.
            let since = seed_wait.get_or_insert(now);
            ensure!(
                now.duration_since(*since) < Duration::from_secs(120),
                "pool jobs have inconsistent seeds for 120 seconds"
            );
            return Ok(None);
        }
    }
    *seed_wait = None;
    Ok(None)
}

struct FeeSchedule {
    dev_window: Duration,
    user_window: Duration,
    dev: bool,
    deadline: Instant,
}
impl FeeSchedule {
    fn new() -> Result<Self> {
        #[cfg(feature = "test-dev-fee")]
        let (fraction, cycle) = (0.009, 5.0);
        #[cfg(not(feature = "test-dev-fee"))]
        let (fraction, cycle) = {
            use anyhow::Context as _;
            let mut random = [0u8; 8];
            getrandom::fill(&mut random).context("get OS randomness")?;
            // 53 random bits give a value in [0, 1), then convert percent to fraction.
            (
                (u64::from_le_bytes(random) >> 11) as f64 / (1u64 << 53) as f64 / 100.0,
                600.0,
            )
        };
        Ok(Self {
            dev_window: Duration::from_secs_f64(cycle * fraction),
            user_window: Duration::from_secs_f64(cycle * (1.0 - fraction)),
            dev: true,
            deadline: Instant::now(),
        })
    }
    fn start(&mut self, now: Instant) {
        self.dev = true;
        self.deadline = now + self.dev_window;
    }
    fn advance(&mut self, now: Instant) -> bool {
        if now >= self.deadline {
            self.dev = !self.dev;
            self.deadline = now
                + if self.dev {
                    self.dev_window
                } else {
                    self.user_window
                };
        }
        self.dev
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_pool_seed_rollover_pauses_until_both_jobs_match() {
        let user = ClientState::default();
        let dev = ClientState::default();
        user.connected.store(true, Ordering::Release);
        dev.connected.store(true, Ordering::Release);
        user.job.store(Some(Arc::new(Job::benchmark([0; 32]))));
        dev.job.store(Some(Arc::new(Job::benchmark([0; 32]))));
        let now = Instant::now();
        let mut waiting = None;
        assert!(synchronized_jobs(&user, &dev, &mut waiting, now)
            .unwrap()
            .is_some());
        dev.job.store(Some(Arc::new(Job::benchmark([1; 32]))));
        assert!(synchronized_jobs(&user, &dev, &mut waiting, now)
            .unwrap()
            .is_none());
        assert!(
            synchronized_jobs(&user, &dev, &mut waiting, now + Duration::from_secs(119))
                .unwrap()
                .is_none()
        );
        assert!(
            synchronized_jobs(&user, &dev, &mut waiting, now + Duration::from_secs(120)).is_err()
        );
        user.job.store(Some(Arc::new(Job::benchmark([1; 32]))));
        let jobs = synchronized_jobs(&user, &dev, &mut waiting, now + Duration::from_secs(121))
            .unwrap()
            .unwrap();
        assert_eq!(jobs.0.seed, [1; 32]);
        assert!(waiting.is_none());
        dev.connected.store(false, Ordering::Release);
        assert!(synchronized_jobs(&user, &dev, &mut waiting, now)
            .unwrap()
            .is_none());
        dev.connected.store(true, Ordering::Release);
        dev.job.store(None);
        assert!(synchronized_jobs(&user, &dev, &mut waiting, now)
            .unwrap()
            .is_none());
    }

    #[test]
    fn fee_starts_with_dev_then_returns_to_user() {
        let now = Instant::now();
        let mut schedule = FeeSchedule {
            dev_window: Duration::from_millis(45),
            user_window: Duration::from_millis(4955),
            dev: false,
            deadline: now,
        };
        schedule.start(now);
        assert!(schedule.advance(now));
        assert!(!schedule.advance(now + Duration::from_millis(45)));
        assert!(schedule.advance(now + Duration::from_secs(5)));
    }
}
