use crate::{
    job::decode_seed,
    proxy::{PoolAddress, Proxy},
    randomx::{MemoryMode, Tuning},
    topology::{self, Core},
};
use anyhow::{bail, ensure, Context, Result};
use clap::Parser;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

#[derive(Parser, Clone, Debug)]
#[command(
    version,
    disable_version_flag = true,
    about = "AI-optimized RandomX miner; built-in developer fee <1%"
)]
pub struct Cli {
    #[arg(short = 'V', long, action = clap::ArgAction::Version)]
    pub version: Option<bool>,
    #[arg(short = 'c', long)]
    pub config: Option<PathBuf>,
    #[arg(short = 'o', long)]
    pub url: Option<String>,
    #[arg(short = 'a', long)]
    pub algo: Option<String>,
    #[arg(long)]
    pub coin: Option<String>,
    #[arg(short = 'u', long)]
    pub user: Option<String>,
    #[arg(short = 'p', long, alias = "password")]
    pub pass: Option<String>,
    #[arg(short = 'O', long, conflicts_with_all = ["user", "pass"])]
    pub userpass: Option<String>,
    #[arg(long)]
    pub rig_id: Option<String>,
    #[arg(short = 't', long)]
    pub threads: Option<usize>,
    /// CPU affinity bitmask (decimal or 0x-prefixed hexadecimal).
    #[arg(long, allow_hyphen_values = true)]
    pub cpu_affinity: Option<String>,
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=100))]
    pub cpu_max_threads_hint: Option<u8>,
    #[arg(long = "randomx-init", allow_hyphen_values = true)]
    pub randomx_init: Option<i32>,
    #[arg(long = "randomx-mode")]
    pub randomx_mode: Option<String>,
    #[arg(long = "randomx-no-numa", alias = "no-numa")]
    pub no_numa: bool,
    #[arg(long)]
    pub huge_pages_jit: bool,
    #[arg(long = "randomx-1gb-pages")]
    pub one_gb_pages: bool,
    #[arg(long)]
    pub nicehash: bool,
    #[arg(long)]
    pub no_huge_pages: bool,
    #[arg(long)]
    pub hugepage_size: Option<u32>,
    /// Enable reserved 2 MiB pages (enabled by default).
    #[arg(long, hide = true, conflicts_with = "no_huge_pages")]
    pub huge_pages: bool,
    /// RxMiner extension: include SMT siblings in automatic selection.
    #[arg(long)]
    pub smt: bool,
    /// RxMiner extension: disable transparent huge-page hints.
    #[arg(long)]
    pub no_thp: bool,
    #[arg(long)]
    pub tls: bool,
    #[arg(long)]
    pub tls_verify: bool,
    #[arg(long)]
    pub tls_ca: Option<String>,
    #[arg(long)]
    pub tls_fingerprint: Option<String>,
    /// SOCKS5 host:port; explicit http:// CONNECT URLs are also supported.
    #[arg(short = 'x', long)]
    pub proxy: Option<String>,
    #[arg(long)]
    pub no_proxy: bool,
    #[arg(long)]
    pub http_host: Option<String>,
    #[arg(long)]
    pub http_port: Option<u16>,
    #[arg(long)]
    pub http_access_token: Option<String>,
    /// Offline hash-count benchmark: 250K, 500K, or 1M through 10M.
    #[arg(long, conflicts_with = "benchmark")]
    pub bench: Option<String>,
    /// RxMiner extension: local benchmark duration in seconds.
    #[arg(long = "benchmark-seconds", alias = "benchmark", value_parser = clap::value_parser!(u64).range(1..=86400))]
    pub benchmark: Option<u64>,
    #[arg(long)]
    pub seed: Option<String>,
    #[arg(long)]
    pub hash: Option<String>,
    #[arg(long)]
    pub print_time: Option<u64>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(short = 'k', long)]
    pub keepalive: bool,
    #[arg(short = 'r', long)]
    pub retries: Option<u32>,
    #[arg(short = 'R', long)]
    pub retry_pause: Option<u64>,
    #[arg(long)]
    pub user_agent: Option<String>,
    #[arg(long)]
    pub cpu_priority: Option<u8>,
    #[arg(long)]
    pub cpu_no_yield: bool,
    #[arg(long)]
    pub argon2_impl: Option<String>,
    #[arg(long)]
    pub no_watch: bool,
    #[arg(long)]
    pub pause_on_battery: bool,
    #[arg(short = 'B', long)]
    pub background: bool,
    #[arg(short = 'l', long)]
    pub log_file: Option<PathBuf>,
    #[arg(short = 'S', long)]
    pub syslog: bool,
    #[arg(long)]
    pub no_color: bool,
    #[arg(long)]
    pub verbose: bool,
    #[arg(long)]
    pub health_print_time: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct TlsOptions {
    pub verify: bool,
    pub ca: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
struct PoolConfig {
    url: String,
    user: String,
    pass: String,
    rig_id: Option<String>,
    algo: Option<String>,
    coin: Option<String>,
    enabled: bool,
    tls: bool,
    tls_fingerprint: Option<String>,
    socks5: Option<String>,
    // Client certificate trust extensions; root "tls" in XMRig config is a server setting.
    tls_verify: bool,
    tls_ca: Option<String>,
    nicehash: bool,
    keepalive: Value,
    daemon: bool,
    self_select: Option<String>,
    submit_to_origin: bool,
}
impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            user: String::new(),
            pass: "x".into(),
            rig_id: None,
            algo: None,
            coin: None,
            enabled: true,
            tls: false,
            tls_fingerprint: None,
            socks5: None,
            tls_verify: false,
            tls_ca: None,
            nicehash: false,
            keepalive: Value::Null,
            daemon: false,
            self_select: None,
            submit_to_origin: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct CpuConfig {
    enabled: bool,
    huge_pages: Value,
    huge_pages_jit: bool,
    memory_pool: Value,
    max_threads_hint: u8,
    hw_aes: Option<bool>,
    priority: Option<u8>,
    #[serde(rename = "yield")]
    yield_cpu: bool,
    argon2_impl: Option<String>,
    #[serde(flatten)]
    profiles: BTreeMap<String, Value>,
    rx: Option<Value>,
    #[serde(rename = "rx/0")]
    rx_0: Option<Value>,
    #[serde(rename = "*")]
    fallback: Option<Value>,
    // RxMiner extensions, not alternate spellings of XMRig fields.
    smt: bool,
    thp: bool,
}
impl Default for CpuConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            huge_pages: Value::Bool(true),
            huge_pages_jit: false,
            memory_pool: Value::Bool(false),
            max_threads_hint: 100,
            hw_aes: None,
            priority: None,
            yield_cpu: true,
            argon2_impl: None,
            profiles: BTreeMap::new(),
            rx: None,
            rx_0: None,
            fallback: None,
            smt: false,
            thp: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
struct RandomxConfig {
    jit: bool,
    secure: bool,
    init: i32,
    init_avx2: i32,
    #[serde(rename = "scratchpad_prefetch_mode")]
    prefetch_mode: u8,
    mode: String,
    numa: bool,
    #[serde(rename = "1gb-pages")]
    gb_pages: bool,
    wrmsr: Value,
    rdmsr: bool,
    #[serde(rename = "cache_qos")]
    cache_qos: bool,
}
impl Default for RandomxConfig {
    fn default() -> Self {
        Self {
            jit: crate::randomx::JIT_SUPPORTED,
            secure: false,
            init: -1,
            init_avx2: -1,
            prefetch_mode: 1,
            mode: "auto".into(),
            numa: true,
            gb_pages: false,
            wrmsr: Value::Bool(false),
            rdmsr: true,
            cache_qos: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
struct HttpOptions {
    enabled: bool,
    host: String,
    port: u16,
    access_token: Option<String>,
    restricted: bool,
}
impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            port: 0,
            access_token: None,
            restricted: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct BenchmarkConfig {
    size: Option<String>,
    seconds: Option<u64>,
    seed: Option<String>,
    algo: Option<String>,
    hash: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
struct FileConfig {
    pools: Vec<PoolConfig>,
    cpu: CpuConfig,
    randomx: RandomxConfig,
    http: HttpOptions,
    benchmark: BenchmarkConfig,
    print_time: u64,
    retries: u32,
    retry_pause: u64,
    user_agent: Option<String>,
    dns: crate::dns::Options,
    tls: crate::http::TlsOptions,
    autosave: bool,
    watch: bool,
    background: bool,
    log_file: Option<PathBuf>,
    syslog: bool,
    colors: bool,
    verbose: u8,
    health_print_time: u64,
    pause_on_battery: bool,
}
impl Default for FileConfig {
    fn default() -> Self {
        Self {
            pools: vec![],
            cpu: CpuConfig::default(),
            randomx: RandomxConfig::default(),
            http: HttpOptions::default(),
            benchmark: BenchmarkConfig::default(),
            print_time: 60,
            retries: 5,
            retry_pause: 5,
            user_agent: None,
            dns: crate::dns::Options::default(),
            tls: crate::http::TlsOptions::default(),
            autosave: true,
            watch: true,
            background: false,
            log_file: None,
            syslog: false,
            colors: true,
            verbose: 0,
            health_print_time: 60,
            pause_on_battery: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PoolOptions {
    pub address: PoolAddress,
    pub user: String,
    pub pass: String,
    pub rig_id: String,
    pub tls: TlsOptions,
    pub proxy: Option<Proxy>,
    pub nicehash: bool,
    pub keepalive: u64,
    pub keepalive_auto: bool,
    pub retries: u32,
    pub retry_pause: u64,
    pub user_agent: String,
    pub dns: crate::dns::Options,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Benchmark {
    Seconds(u64),
    Hashes(u64),
}

#[derive(Clone, Debug)]
pub struct Options {
    pub source_cli: Cli,
    pub http_restricted: bool,
    pub server_tls: crate::http::TlsOptions,
    pub pool: Option<PoolOptions>,
    pub pools: Vec<PoolOptions>,
    pub autosave: bool,
    pub watch: bool,
    pub background: bool,
    pub log_file: Option<PathBuf>,
    pub syslog: bool,
    pub colors: bool,
    pub verbose: bool,
    pub health_print_time: u64,
    pub pause_on_battery: bool,
    pub msr: Vec<crate::msr::Entry>,
    pub restore_msr: bool,
    pub tuning: Tuning,
    pub huge_pages_jit: bool,
    pub memory_pool: Option<usize>,
    pub prefetch_mode: u8,
    pub priority: Option<u8>,
    pub yield_cpu: bool,
    pub threads: usize,
    pub profile: Option<Vec<Option<usize>>>,
    pub max_threads_hint: u8,
    pub init_threads: Option<usize>,
    pub numa: bool,
    pub smt: bool,
    pub thp: bool,
    pub huge_pages: bool,
    pub http_host: String,
    pub http_port: u16,
    pub http_access_token: Option<String>,
    pub benchmark: Option<Benchmark>,
    pub expected_hash: Option<u64>,
    pub seed: [u8; 32],
    pub print_time: u64,
    pub dry_run: bool,
}

fn algorithm(value: Option<&str>) -> Result<()> {
    ensure!(
        value.is_none_or(|v| v == "rx/0" || v == "rx"),
        "only the rx/0 algorithm is supported"
    );
    Ok(())
}

fn benchmark_size(value: &str) -> Result<u64> {
    let value = value.to_ascii_uppercase();
    match value.as_str() {
        "250K" => Ok(250_000),
        "500K" => Ok(500_000),
        _ => {
            let n = value
                .strip_suffix('M')
                .context("--bench expects 250K, 500K or 1M..10M")?
                .parse::<u64>()
                .context("invalid benchmark size")?;
            ensure!((1..=10).contains(&n), "benchmark size must be 1M..10M");
            Ok(n * 1_000_000)
        }
    }
}

fn affinity_mask(value: &str) -> Result<Option<Vec<usize>>> {
    if value == "-1" {
        return Ok(None);
    }
    let mask = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16)
    } else {
        value.parse::<u64>()
    }
    .context("invalid CPU affinity mask")?;
    ensure!(mask != 0, "CPU affinity mask cannot be empty");
    Ok(Some(
        (0..64).filter(|cpu| mask & (1u64 << cpu) != 0).collect(),
    ))
}

fn profile(value: &Value) -> Result<Vec<Option<usize>>> {
    let values = if let Some(array) = value.as_array() {
        let mut result = Vec::new();
        for entry in array {
            let entry = if let Some(pair) = entry.as_array() {
                ensure!(
                    pair.len() == 2 && pair[0].as_u64() == Some(1),
                    "RandomX thread intensity must be 1"
                );
                &pair[1]
            } else {
                entry
            };
            let id = entry
                .as_i64()
                .context("CPU profile entries must be CPU IDs or -1")?;
            ensure!((-1..1 << 20).contains(&id), "CPU profile ID out of range");
            result.push((id >= 0).then_some(id as usize));
        }
        result
    } else if value.is_object() {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Definition {
            threads: usize,
            affinity: Option<Value>,
            intensity: Option<u8>,
        }
        let definition: Definition =
            serde_json::from_value(value.clone()).context("invalid CPU profile")?;
        ensure!(
            definition.intensity.is_none_or(|v| v == 1),
            "RandomX thread intensity must be 1"
        );
        ensure!(
            (1..=1024).contains(&definition.threads),
            "CPU profile needs 1..1024 threads"
        );
        let mask = definition
            .affinity
            .map(|v| match v {
                Value::String(s) => affinity_mask(&s),
                Value::Number(n) => affinity_mask(&n.to_string()),
                _ => bail!("CPU profile affinity must be a bitmask or -1"),
            })
            .transpose()?
            .flatten();
        (0..definition.threads)
            .map(|i| mask.as_ref().and_then(|cpus| cpus.get(i).copied()))
            .collect()
    } else {
        bail!("cpu.rx must be a thread array or object; disabling rx/0 leaves no mining backend")
    };
    ensure!(
        (1..=1024).contains(&values.len()),
        "CPU profile needs 1..1024 threads"
    );
    Ok(values)
}

impl Options {
    pub fn from_json(cli: Cli, bytes: &[u8]) -> Result<Self> {
        Self::resolve(
            cli,
            serde_json::from_slice(bytes).context("invalid configuration")?,
        )
    }
    pub fn from_cli(cli: Cli) -> Result<Self> {
        let file = if let Some(path) = &cli.config {
            serde_json::from_slice(&fs::read(path).with_context(|| format!("read {}", path.display()))?)
                .context("invalid configuration (use pools, cpu and randomx sections; unsupported options are rejected)")?
        } else {
            FileConfig::default()
        };
        Self::resolve(cli, file)
    }

    fn resolve(cli: Cli, mut file: FileConfig) -> Result<Self> {
        let source_cli = cli.clone();
        file.pools.retain(|pool| pool.enabled);
        if file.pools.len() > 1 {
            let pools = std::mem::take(&mut file.pools);
            let mut options = None;
            for (index, pool) in pools.into_iter().enumerate() {
                let mut single = file.clone();
                single.pools.push(pool);
                let mut pool_cli = cli.clone();
                if index > 0 {
                    pool_cli.url = None;
                }
                let next = Self::resolve(pool_cli, single)?;
                if let Some(previous) = &mut options {
                    let previous: &mut Self = previous;
                    previous.pools.extend(next.pools);
                } else {
                    options = Some(next);
                }
            }
            return options.context("no enabled pools");
        }
        file.dns.validate()?;
        ensure!(file.cpu.enabled, "CPU mining is the only supported backend");
        ensure!(
            file.randomx.prefetch_mode <= 3,
            "randomx.scratchpad_prefetch_mode must be 0..3"
        );
        ensure!(
            !file.randomx.cache_qos,
            "randomx.cache_qos is not supported"
        );
        let msr = crate::msr::parse(&file.randomx.wrmsr)?;
        let mode = cli.randomx_mode.as_deref().unwrap_or(&file.randomx.mode);
        let mode = match mode {
            "auto" => MemoryMode::Auto,
            "fast" => MemoryMode::Fast,
            "light" => MemoryMode::Light,
            _ => bail!("randomx.mode must be auto, fast or light"),
        };
        let implementation = cli
            .argon2_impl
            .as_deref()
            .or(file.cpu.argon2_impl.as_deref());
        let argon2 = match implementation.map(str::to_ascii_lowercase).as_deref() {
            None | Some("auto") => None,
            Some("x86_64") => Some(0),
            Some("ssse3") => Some(32),
            Some("avx2") => Some(64),
            _ => bail!("Argon2 implementation must be auto, x86_64, SSSE3 or AVX2"),
        };
        ensure!(
            (-1..=1).contains(&file.randomx.init_avx2),
            "randomx.init-avx2 must be -1, 0 or 1"
        );
        ensure!(file.randomx.init_avx2 != 1,
            "AVX2 dataset initialization is not implemented; select the cache kernel with cpu.argon2-impl");
        let memory_pool = match &file.cpu.memory_pool {
            Value::Bool(false) => Some(0),
            Value::Bool(true) => None,
            Value::Number(n) => Some(
                n.as_u64()
                    .filter(|n| *n <= 1024)
                    .context("cpu.memory-pool must be 0..1024 pages")? as usize,
            ),
            _ => bail!("cpu.memory-pool must be boolean or a page count"),
        };
        let tuning = Tuning {
            jit: file.randomx.jit,
            secure: file.randomx.secure,
            mode,
            one_gb_pages: file.randomx.gb_pages || cli.one_gb_pages,
            hw_aes: file.cpu.hw_aes,
            argon2,
        };
        tuning.validate()?;
        let priority = cli.cpu_priority.or(file.cpu.priority);
        ensure!(priority.is_none_or(|v| v <= 5), "cpu.priority must be 0..5");
        for (name, value) in &file.cpu.profiles {
            ensure!(
                !name.contains('_'),
                "unknown CPU setting {name}; use hyphenated option names"
            );
            ensure!(
                value.is_array()
                    || value.is_object()
                    || value.is_string()
                    || value == &Value::Bool(false),
                "invalid CPU profile {name}"
            );
        }
        let init = cli.randomx_init.unwrap_or(file.randomx.init);
        ensure!(
            (-1..=1024).contains(&init),
            "randomx.init must be -1, 0 or 1..1024"
        );
        let hint = cli
            .cpu_max_threads_hint
            .unwrap_or(file.cpu.max_threads_hint);
        ensure!(
            (1..=100).contains(&hint),
            "cpu.max-threads-hint must be 1..100"
        );
        let mut huge_pages = match file.cpu.huge_pages {
            Value::Bool(v) => v,
            Value::Number(ref n) if n.as_u64() == Some(2048) => true,
            _ => bail!("cpu.huge-pages must be boolean or 2048 (KiB)"),
        };
        if let Some(size) = cli.hugepage_size {
            ensure!(size == 2048, "only 2048 KiB huge pages are supported");
            huge_pages = true;
        }
        if cli.huge_pages {
            huge_pages = true;
        }
        if cli.no_huge_pages {
            huge_pages = false;
        }
        let selected_profile = file
            .cpu
            .rx_0
            .as_ref()
            .or(file.cpu.rx.as_ref())
            .or(file.cpu.fallback.as_ref());
        let mut selected_profile = selected_profile
            .map(|value| {
                let mut value = value;
                let mut visited = BTreeSet::new();
                while let Some(name) = value.as_str() {
                    ensure!(visited.insert(name), "cyclic CPU profile reference {name}");
                    value = file
                        .cpu
                        .profiles
                        .get(name)
                        .with_context(|| format!("unknown CPU profile {name}"))?;
                }
                profile(value)
            })
            .transpose()?;
        let threads = cli.threads.unwrap_or(0);
        ensure!(threads <= 1024, "threads must be 0..1024");
        if cli.threads.is_some() {
            selected_profile = (threads > 0).then(|| vec![None; threads]);
        }
        if let Some(mask) = &cli.cpu_affinity {
            let mask = affinity_mask(mask)?;
            let count = if threads > 0 {
                threads
            } else {
                selected_profile
                    .as_ref()
                    .map(Vec::len)
                    .or_else(|| mask.as_ref().map(Vec::len))
                    .context("--cpu-affinity=-1 requires --threads or a CPU profile")?
            };
            selected_profile = Some(
                (0..count)
                    .map(|i| mask.as_ref().and_then(|cpus| cpus.get(i).copied()))
                    .collect(),
            );
        }
        let bench = &file.benchmark;
        algorithm(bench.algo.as_deref())?;
        let benchmark = if let Some(seconds) = cli.benchmark {
            Some(Benchmark::Seconds(seconds))
        } else if let Some(size) = &cli.bench {
            Some(Benchmark::Hashes(benchmark_size(size)?))
        } else {
            ensure!(
                bench.size.is_none() || bench.seconds.is_none(),
                "benchmark.size and benchmark.seconds are mutually exclusive"
            );
            if let Some(size) = &bench.size {
                Some(Benchmark::Hashes(benchmark_size(size)?))
            } else {
                bench.seconds.map(Benchmark::Seconds)
            }
        };
        if let Some(Benchmark::Seconds(n)) = benchmark {
            ensure!((1..=86400).contains(&n), "benchmark duration out of range");
        }
        let seed = cli
            .seed
            .as_ref()
            .or(bench.seed.as_ref())
            .map(|s| decode_seed(s))
            .transpose()?
            .unwrap_or([0; 32]);
        file.pools.retain(|pool| pool.enabled);
        let mut pool = file.pools.pop().unwrap_or_default();
        if let Some(v) = cli.url {
            pool.url = v;
        }
        if let Some(v) = cli.user {
            pool.user = v;
        }
        if let Some(v) = cli.pass {
            pool.pass = v;
        }
        if let Some(v) = cli.userpass {
            let (user, pass) = v.split_once(':').context("--userpass must be USER:PASS")?;
            pool.user = user.into();
            pool.pass = pass.into();
        }
        if let Some(v) = cli.rig_id {
            pool.rig_id = Some(v);
        }
        algorithm(cli.algo.as_deref().or(pool.algo.as_deref()))?;
        let coin = cli.coin.as_deref().or(pool.coin.as_deref());
        ensure!(
            coin.is_none_or(|s| s.eq_ignore_ascii_case("monero") || s.eq_ignore_ascii_case("xmr")),
            "only Monero (rx/0) coin selection is supported"
        );
        ensure!(
            !pool.daemon && pool.self_select.is_none() && !pool.submit_to_origin,
            "daemon and self-select modes are not supported"
        );
        let keepalive_auto = !cli.keepalive && pool.keepalive.is_null();
        let keepalive = if cli.keepalive {
            60
        } else {
            match pool.keepalive {
                Value::Null => 0,
                Value::Bool(true) => 60,
                Value::Bool(false) => 0,
                Value::Number(n) => n.as_u64().context("keepalive must be nonnegative")?,
                _ => bail!("keepalive must be null, boolean or seconds"),
            }
        };
        ensure!(keepalive <= 86400, "keepalive must be 0..86400 seconds");
        let retries = cli.retries.unwrap_or(file.retries);
        let retry_pause = cli.retry_pause.unwrap_or(file.retry_pause);
        ensure!((1..=1000).contains(&retries), "retries must be 1..1000");
        ensure!(retry_pause <= 86400, "retry-pause must be 0..86400 seconds");
        let user_agent = cli
            .user_agent
            .or(file.user_agent)
            .unwrap_or_else(|| concat!("RxMiner/", env!("CARGO_PKG_VERSION")).into());
        let ca = cli.tls_ca.or(pool.tls_ca).unwrap_or_default();
        let fingerprint = cli
            .tls_fingerprint
            .or(pool.tls_fingerprint)
            .unwrap_or_default()
            .replace(':', "");
        ensure!(
            fingerprint.is_empty()
                || (fingerprint.len() == 64 && hex::decode(&fingerprint).is_ok()),
            "TLS fingerprint must be a SHA-256 hex digest"
        );
        let tls = TlsOptions {
            verify: cli.tls_verify || pool.tls_verify || !ca.is_empty(),
            ca,
            fingerprint: fingerprint.to_ascii_lowercase(),
        };
        let pool = if benchmark.is_none() {
            ensure!(!pool.user.is_empty(), "--user or pools[].user is required");
            let mut address = PoolAddress::parse(&pool.url)?;
            if cli.tls || pool.tls {
                address.tls = true;
                address.url = format!("stratum+ssl://{}", address.authority());
            }
            if let Some(proxy) = &pool.socks5 {
                ensure!(
                    !proxy.contains("://")
                        || proxy.starts_with("socks5://")
                        || proxy.starts_with("socks5h://"),
                    "pools[].socks5 requires a SOCKS5 endpoint"
                );
            }
            let explicit_proxy = cli.proxy.or(pool.socks5).filter(|s| !s.is_empty());
            let proxy = if let Some(proxy) = explicit_proxy {
                Some(Proxy::parse(&if proxy.contains("://") {
                    proxy
                } else {
                    format!("socks5h://{proxy}")
                })?)
            } else if cli.no_proxy {
                None
            } else {
                Proxy::from_environment(&address)?
            };
            Some(PoolOptions {
                address,
                user: pool.user,
                pass: pool.pass,
                rig_id: pool.rig_id.unwrap_or_default(),
                tls,
                proxy,
                nicehash: pool.nicehash || cli.nicehash,
                keepalive,
                keepalive_auto,
                retries,
                retry_pause,
                user_agent,
                dns: file.dns,
            })
        } else {
            None
        };
        let http_enabled = cli
            .http_port
            .map(|port| port > 0)
            .unwrap_or(file.http.enabled);
        let token = cli
            .http_access_token
            .or(file.http.access_token)
            .filter(|s| !s.is_empty());
        ensure!(
            token.as_ref().is_none_or(
                |s| s.is_ascii() && !s.bytes().any(|b| b.is_ascii_control() || b == b' ')
            ),
            "HTTP access token must contain visible ASCII without spaces"
        );
        ensure!(
            !http_enabled || file.http.restricted || token.is_some(),
            "unrestricted HTTP API requires an access token"
        );
        let http_port = cli.http_port.unwrap_or(file.http.port);
        ensure!(
            !http_enabled || http_port > 0,
            "enabled HTTP API requires a nonzero port"
        );
        file.tls.acceptor()?;
        let expected_hash = cli
            .hash
            .or(file.benchmark.hash)
            .map(|value| -> Result<u64> {
                ensure!(
                    value.len() == 16 && value.bytes().all(|b| b.is_ascii_hexdigit()),
                    "benchmark hash must contain 16 hexadecimal digits"
                );
                ensure!(
                    matches!(benchmark, Some(Benchmark::Hashes(_))),
                    "benchmark hash verification requires --bench or benchmark.size"
                );
                Ok(u64::from_str_radix(&value, 16)?)
            })
            .transpose()?;
        let print_time = cli.print_time.unwrap_or(file.print_time);
        ensure!(print_time <= 86400, "print-time must be 0..86400 seconds");
        let options = Self {
            source_cli,
            http_restricted: file.http.restricted,
            server_tls: file.tls,
            pools: pool.iter().cloned().collect(),
            autosave: file.autosave,
            watch: file.watch && !cli.no_watch,
            background: file.background || cli.background,
            log_file: cli.log_file.or(file.log_file),
            syslog: file.syslog || cli.syslog,
            colors: file.colors && !cli.no_color,
            verbose: file.verbose > 0 || cli.verbose,
            health_print_time: cli.health_print_time.unwrap_or(file.health_print_time),
            pause_on_battery: file.pause_on_battery || cli.pause_on_battery,
            pool,
            msr,
            restore_msr: file.randomx.rdmsr,
            tuning,
            huge_pages_jit: file.cpu.huge_pages_jit || cli.huge_pages_jit,
            memory_pool,
            prefetch_mode: file.randomx.prefetch_mode,
            priority,
            yield_cpu: file.cpu.yield_cpu && !cli.cpu_no_yield,
            threads,
            profile: selected_profile,
            max_threads_hint: hint,
            init_threads: (init > 0).then_some(init as usize),
            numa: file.randomx.numa && !cli.no_numa,
            smt: file.cpu.smt || cli.smt,
            thp: file.cpu.thp && !cli.no_thp,
            huge_pages,
            http_host: cli.http_host.unwrap_or(file.http.host),
            http_port: if http_enabled { http_port } else { 0 },
            http_access_token: token,
            benchmark,
            expected_hash,
            seed,
            print_time,
            dry_run: cli.dry_run,
        };
        options.validate_platform()?;
        Ok(options)
    }

    fn validate_platform(&self) -> Result<()> {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            ensure!(
                self.msr.is_empty(),
                "MSR tuning currently requires Linux x86-64"
            );
        }
        if !cfg!(target_os = "linux") {
            ensure!(
                !self.tuning.one_gb_pages,
                "1 GiB pages currently require Linux"
            );
            ensure!(
                !self.pause_on_battery,
                "battery monitoring currently requires Linux"
            );
        }
        if !cfg!(all(target_arch = "x86_64", any(unix, windows))) {
            ensure!(
                self.prefetch_mode == 1,
                "custom JIT prefetch modes currently require the x86-64 backend"
            );
        }
        if cfg!(windows) {
            ensure!(
                !self.background,
                "background mode is unavailable on Windows"
            );
            ensure!(
                !self.syslog,
                "syslog is unavailable on Windows; use log-file"
            );
        }
        if cfg!(target_os = "macos") {
            ensure!(
                !self
                    .profile
                    .as_ref()
                    .is_some_and(|p| p.iter().any(Option::is_some)),
                "macOS does not expose hard CPU affinity; use unbound workers (-1)"
            );
        }
        Ok(())
    }

    pub fn select_cores(&self, all: &[Core], quota: Option<usize>) -> Result<Vec<Core>> {
        ensure!(!all.is_empty(), "no usable CPU cores found");
        if let Some(profile) = &self.profile {
            return profile
                .iter()
                .enumerate()
                .map(|(index, id)| {
                    let mut core = match id {
                        Some(id) => all
                            .iter()
                            .find(|core| core.cpu == *id)
                            .with_context(|| {
                                format!("CPU {id} is unavailable in the current affinity mask")
                            })?
                            .clone(),
                        None => all[index % all.len()].clone(),
                    };
                    core.pinned = id.is_some();
                    Ok(core)
                })
                .collect();
        }
        let capacity = topology::select(all, 0, self.smt).len();
        let count = if self.threads > 0 {
            self.threads
        } else {
            all.len()
                .min(quota.unwrap_or(all.len()))
                .saturating_mul(usize::from(self.max_threads_hint))
                .div_ceil(100)
                .min(capacity)
                .max(1)
        };
        ensure!(count <= capacity, "requested thread count exceeds selected CPU capacity; use --smt or an explicit cpu.rx profile");
        Ok(topology::select(all, count, self.smt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(args: &[&str], json: &str) -> Result<Options> {
        Options::resolve(Cli::try_parse_from(args)?, serde_json::from_str(json)?)
    }
    #[test]
    fn standard_pool_tls_proxy_and_cli_overrides() {
        let opts = options(&["rxminer", "-o", "new.example:443", "--tls", "-O", "user:password:tail", "-x", "proxy.example:1080", "--no-proxy"],
            r#"{"pools":[{"url":"old.example:3333","user":"old","rig-id":"rig","tls-fingerprint":null,"algo":"rx/0"}]}"#).unwrap();
        let pool = opts.pool.unwrap();
        assert_eq!(pool.address.host, "new.example");
        assert!(pool.address.tls);
        assert_eq!(pool.pass, "password:tail");
        assert_eq!(pool.rig_id, "rig");
        assert_eq!(pool.proxy.unwrap().label(), "socks5h://proxy.example:1080");
    }
    #[test]
    fn canonical_config_and_benchmark() {
        let opts = options(&["rxminer", "--bench", "1M", "--randomx-no-numa", "--no-huge-pages"],
            r#"{"cpu":{"rx":[-1,-1],"huge-pages":2048},"randomx":{"init":2},"http":{"enabled":false,"port":2333}}"#).unwrap();
        assert!(opts.pool.is_none());
        assert_eq!(opts.benchmark, Some(Benchmark::Hashes(1_000_000)));
        assert_eq!(opts.profile, Some(vec![None, None]));
        assert_eq!(opts.init_threads, Some(2));
        assert!(!opts.numa && !opts.huge_pages);
        assert_eq!(opts.http_port, 0);
        assert!(
            options(&["rxminer", "--benchmark", "1"], "{}")
                .unwrap()
                .huge_pages
        );
    }
    #[test]
    fn cli_precedence_and_compatibility_aliases() {
        let opts = options(
            &[
                "rxminer",
                "--benchmark=2",
                "-t",
                "4",
                "--no-numa",
                "--http-port",
                "1234",
                "--print-time",
                "0",
            ],
            r#"{"cpu":{"rx":[0,1]},"benchmark":{"size":"10M"},"http":{"enabled":false}}"#,
        )
        .unwrap();
        assert_eq!(opts.threads, 4);
        assert_eq!(opts.profile, Some(vec![None; 4]));
        assert_eq!(opts.benchmark, Some(Benchmark::Seconds(2)));
        assert_eq!(opts.http_port, 1234);
        assert_eq!(opts.print_time, 0);
        assert!(!opts.numa);
    }
    #[test]
    fn benchmark_checksums_are_local_and_require_a_fixed_count() {
        let opts = options(
            &["rxminer", "--bench", "250K", "--hash", "fedcba9876543210"],
            "{}",
        )
        .unwrap();
        assert_eq!(opts.expected_hash, Some(0xfedcba9876543210));
        assert!(options(
            &["rxminer", "--benchmark=1", "--hash", "0000000000000000"],
            "{}"
        )
        .is_err());
        assert!(options(&["rxminer", "--bench", "250K", "--hash", "wrong"], "{}").is_err());
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn local_options_and_profile_aliases_are_resolved() {
        let opts = options(
            &["rxminer", "--benchmark=1"],
            r#"{
            "cpu":{"rx":"small", "small":[-1,-1], "hw-aes":false,
                "priority":1,"yield":false,"memory-pool":4,"huge-pages-jit":true},
            "randomx":{"mode":"light","1gb-pages":true,"scratchpad_prefetch_mode":3},
            "autosave":false,"watch":false,"health-print-time":0
        }"#,
        )
        .unwrap();
        assert_eq!(opts.profile, Some(vec![None, None]));
        assert_eq!(opts.tuning.mode, MemoryMode::Light);
        assert_eq!(opts.tuning.hw_aes, Some(false));
        assert!(opts.tuning.one_gb_pages && opts.huge_pages_jit);
        assert_eq!(opts.memory_pool, Some(4));
        assert_eq!(opts.prefetch_mode, 3);
        assert!(!opts.autosave && !opts.watch && !opts.yield_cpu);
        for config in [
            r#"{"cpu":{"rx":"a","a":"b","b":"a"}}"#,
            r#"{"cpu":{"rx":"missing"}}"#,
            r#"{"cpu":{"memory-pool":-1}}"#,
            r#"{"randomx":{"scratchpad_prefetch_mode":4}}"#,
            r#"{"randomx":{"init-avx2":1}}"#,
            r#"{"benchmark":{"verify":"remote-id"}}"#,
        ] {
            assert!(
                options(&["rxminer", "--benchmark=1"], config).is_err(),
                "{config}"
            );
        }
    }

    #[test]
    fn backup_pools_preserve_per_pool_network_options() {
        let opts = options(
            &["rxminer", "--no-proxy"],
            r#"{
            "pools":[{"url":"first.invalid","user":"one","keepalive":1,"nicehash":true},
                     {"url":"disabled.invalid","enabled":false},
                     {"url":"second.invalid","user":"two","keepalive":false}],
            "retries":3,"retry-pause":0,"user-agent":"local-agent"
        }"#,
        )
        .unwrap();
        assert_eq!(opts.pools.len(), 2);
        assert_eq!(opts.pools[0].user, "one");
        assert_eq!(opts.pools[1].user, "two");
        assert_eq!(opts.pools[0].keepalive, 1);
        assert_eq!(opts.pools[1].keepalive, 0);
        assert!(opts.pools[0].nicehash);
        assert!(!opts.pools[1].nicehash);
        assert_eq!(opts.pools[1].retries, 3);
        assert_eq!(opts.pools[1].user_agent, "local-agent");
    }

    #[test]
    fn keepalive_defaults_to_negotiation_and_respects_explicit_overrides() {
        for (setting, automatic, seconds) in [
            ("null", true, 0),
            ("false", false, 0),
            ("true", false, 60),
            ("7", false, 7),
        ] {
            let config = format!(
                r#"{{"pools":[{{"url":"localhost:3333","user":"test","keepalive":{setting}}}]}}"#
            );
            let pool = options(&["rxminer", "--no-proxy"], &config)
                .unwrap()
                .pool
                .unwrap();
            assert_eq!(pool.keepalive_auto, automatic);
            assert_eq!(pool.keepalive, seconds);
            let forced = options(&["rxminer", "--no-proxy", "-k"], &config)
                .unwrap()
                .pool
                .unwrap();
            assert!(!forced.keepalive_auto);
            assert_eq!(forced.keepalive, 60);
        }
        let pool = options(
            &[
                "rxminer",
                "--no-proxy",
                "-o",
                "localhost:3333",
                "-u",
                "test",
            ],
            "{}",
        )
        .unwrap()
        .pool
        .unwrap();
        assert!(pool.keepalive_auto);
    }

    #[test]
    fn rejects_unsupported_and_misspelled_configuration() {
        for json in [
            r#"{"cpu":{"huge_pages":false}}"#,
            r#"{"pool":"pool.example"}"#,
            r#"{"cpu":{"enabled":false}}"#,
            r#"{"donate-level":0}"#,
            r#"{"dev-pool":null}"#,
            r#"{"dev-pool":{"url":"pool.invalid:3333"}}"#,
            r#"{"cpu":{"rx":[[2,0]]}}"#,
            r#"{"benchmark":{"submit":true}}"#,
            r#"{"http":{"enabled":true,"port":0}}"#,
        ] {
            assert!(
                options(&["rxminer", "--benchmark=1"], json).is_err(),
                "{json}"
            );
        }
        for args in [
            vec!["rxminer", "-v"],
            vec!["rxminer", "--dev-fee", "0"],
            vec!["rxminer", "--donate-level", "0"],
            vec!["rxminer", "--dev-pool", "pool.invalid:3333"],
            vec!["rxminer", "-t", "2junk"],
            vec!["rxminer", "--benchmark", "0"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
        for flag in ["-V", "--version"] {
            assert_eq!(
                Cli::try_parse_from(["rxminer", flag]).unwrap_err().kind(),
                clap::error::ErrorKind::DisplayVersion
            );
        }
    }
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn profiles_preserve_affinity_and_unbound_workers() {
        let all = vec![
            Core {
                cpu: 2,
                node: 0,
                smt: false,
                pinned: true,
            },
            Core {
                cpu: 5,
                node: 1,
                smt: true,
                pinned: true,
            },
        ];
        let opts = options(&["rxminer", "--benchmark=1"], r#"{"cpu":{"rx":[5,-1,5]}}"#).unwrap();
        let cores = opts.select_cores(&all, Some(1)).unwrap();
        assert_eq!(cores.len(), 3);
        assert_eq!((cores[0].cpu, cores[0].pinned), (5, true));
        assert!(!cores[1].pinned);
        assert_eq!(cores[2].cpu, 5);
        assert!(opts.select_cores(&all[..1], None).is_err());
        assert_eq!(
            profile(&serde_json::json!({"threads":3,"affinity":"0x24"})).unwrap(),
            [Some(2), Some(5), None]
        );
        let opts = options(
            &[
                "rxminer",
                "--benchmark=1",
                "--threads=3",
                "--cpu-affinity=0x24",
            ],
            "{}",
        )
        .unwrap();
        assert_eq!(opts.profile, Some(vec![Some(2), Some(5), None]));
    }
    #[test]
    fn unavailable_platform_options_are_rejected() {
        if cfg!(target_os = "macos") {
            assert!(options(&["rxminer", "--benchmark=1"], r#"{"cpu":{"rx":[0]}}"#).is_err());
            assert!(options(&["rxminer", "--benchmark=1"], r#"{"cpu":{"rx":[-1]}}"#).is_ok());
        }
        if cfg!(windows) {
            for flag in ["--background", "--syslog"] {
                assert!(options(&["rxminer", "--benchmark=1", flag], "{}").is_err());
            }
        }
        if !cfg!(target_os = "linux") {
            assert!(options(&["rxminer", "--benchmark=1", "--randomx-1gb-pages"], "{}").is_err());
        }
    }
    #[test]
    fn benchmark_sizes_and_example_are_valid() {
        for (text, n) in [
            ("250K", 250_000),
            ("500K", 500_000),
            ("1M", 1_000_000),
            ("10M", 10_000_000),
        ] {
            assert_eq!(benchmark_size(text).unwrap(), n);
        }
        for text in ["1", "20M", "0M", "100K"] {
            assert!(benchmark_size(text).is_err());
        }
        options(
            &["rxminer", "--dry-run", "--no-proxy"],
            include_str!("../config.example.json"),
        )
        .unwrap();
    }

    #[test]
    fn automatic_thread_hint_and_cli_page_size_use_standard_meanings() {
        let all: Vec<_> = (0..4)
            .map(|cpu| Core {
                cpu,
                node: 0,
                smt: cpu >= 2,
                pinned: true,
            })
            .collect();
        let opts = options(
            &["rxminer", "--benchmark=1", "--cpu-max-threads-hint=50"],
            "{}",
        )
        .unwrap();
        assert_eq!(opts.select_cores(&all, None).unwrap().len(), 2);
        assert_eq!(opts.select_cores(&all, Some(2)).unwrap().len(), 1);
        let opts = options(
            &[
                "rxminer",
                "--benchmark=1",
                "--threads=3",
                "--cpu-max-threads-hint=25",
            ],
            "{}",
        )
        .unwrap();
        assert_eq!(opts.select_cores(&all, Some(1)).unwrap().len(), 3);
        let opts = options(
            &["rxminer", "--benchmark=1", "--hugepage-size=2048"],
            r#"{"cpu":{"huge-pages":false}}"#,
        )
        .unwrap();
        assert!(opts.huge_pages);
    }
}
