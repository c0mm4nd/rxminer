use crate::{
    config::{Cli, Options},
    miner, topology,
};
use anyhow::{Context, Result};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

// Called by main before signal registration or creation of any worker thread.
pub fn background() -> Result<bool> {
    // SAFETY: main calls this while single-threaded; both branches resume ordinary Rust execution.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(std::io::Error::last_os_error()).context("fork background process");
    }
    if pid > 0 {
        return Ok(true);
    }
    // SAFETY: this child is not a process group leader after fork.
    if unsafe { libc::setsid() } < 0 {
        return Err(std::io::Error::last_os_error()).context("detach session");
    }
    let null = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")?;
    use std::os::fd::AsRawFd;
    for target in [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        // SAFETY: descriptor belongs to the live null file; targets are standard descriptors.
        if unsafe { libc::dup2(null.as_raw_fd(), target) } < 0 {
            return Err(std::io::Error::last_os_error()).context("redirect standard stream");
        }
    }
    Ok(false)
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let temporary = path.with_extension(format!(
        "rxminer-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn autosave(options: &Options, cli: &Cli) -> Result<()> {
    if !options.autosave || options.profile.is_some() {
        return Ok(());
    }
    let Some(path) = &cli.config else {
        return Ok(());
    };
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let cores = options.select_cores(&topology::discover()?, topology::cpu_quota())?;
    let root = json
        .as_object_mut()
        .context("configuration must be an object")?;
    let cpu = root
        .entry("cpu")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .context("cpu must be an object")?;
    cpu.insert(
        "rx".into(),
        serde_json::json!(cores
            .iter()
            .map(|c| if c.pinned { c.cpu as i64 } else { -1 })
            .collect::<Vec<_>>()),
    );
    atomic_write(path, &serde_json::to_vec_pretty(&json)?)
}

pub fn run(mut options: Options, cli: Cli, stop: Arc<AtomicBool>) -> Result<()> {
    let mut pool = 0;
    let mut attempts = 0;
    let mut observed = cli.config.as_ref().and_then(|p| fs::read(p).ok());
    let background = options.background;
    loop {
        if options.watch {
            if let Some(bytes) = cli.config.as_ref().and_then(|p| fs::read(p).ok()) {
                if observed.as_ref() != Some(&bytes) {
                    observed = Some(bytes.clone());
                    match reload_options(&cli, &bytes, background) {
                        Ok(replacement) => {
                            options = replacement;
                            pool = 0;
                            attempts = 0;
                        }
                        Err(error) => crate::status!("[config] reload rejected: {error:#}"),
                    }
                }
            }
        }
        crate::logging::configure(&options)?;
        crate::randomx::set_thp(options.thp);
        crate::randomx::configure_jit(options.huge_pages_jit, options.prefetch_mode);
        let memory_pool = match options.memory_pool {
            Some(pages) => pages,
            None => options
                .select_cores(&topology::discover()?, topology::cpu_quota())?
                .len(),
        };
        crate::randomx::set_memory_pool(memory_pool);
        autosave(&options, &cli)?;
        if options.autosave && options.profile.is_none() {
            if let Some(bytes) = cli.config.as_ref().and_then(|p| fs::read(p).ok()) {
                options = reload_options(&cli, &bytes, background)?;
                observed = Some(bytes);
            }
        }
        let local_stop = Arc::new(AtomicBool::new(false));
        let control = crate::control::Control::new(local_stop.clone());
        let monitor_stop = Arc::new(AtomicBool::new(false));
        let reloaded = Arc::new(Mutex::new(None));
        let watch = options.watch;
        let old = observed.clone();
        let monitor = {
            let stop = stop.clone();
            let local_stop = local_stop.clone();
            let monitor_stop = monitor_stop.clone();
            let reloaded = reloaded.clone();
            let cli = cli.clone();
            let control = control.clone();
            thread::Builder::new()
                .name("runtime-monitor".into())
                .spawn(move || {
                    let monitor_control = crate::control::Control::new(local_stop.clone());
                    let _panic = monitor_control.watch_thread("runtime monitor");
                    let mut previous = old;
                    while !monitor_stop.load(Ordering::Acquire) {
                        if stop.load(Ordering::Acquire) {
                            local_stop.store(true, Ordering::Release);
                            break;
                        }
                        let requested = control.reload.swap(false, Ordering::AcqRel);
                        if watch || requested {
                            if let Some(path) = &cli.config {
                                if let Ok(bytes) = fs::read(path) {
                                    if requested || previous.as_ref() != Some(&bytes) {
                                        previous = Some(bytes.clone());
                                        let replacement = reload_options(&cli, &bytes, background);
                                        match replacement {
                                            Ok(o) => {
                                                *reloaded
                                                    .lock()
                                                    .unwrap_or_else(|p| p.into_inner()) = Some(o);
                                                local_stop.store(true, Ordering::Release);
                                                break;
                                            }
                                            Err(error) => crate::status!(
                                                "[config] reload rejected: {error:#}"
                                            ),
                                        }
                                    }
                                }
                            }
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                })?
        };
        let mut current = options.clone();
        current.pool = options.pools.get(pool).cloned();
        if options.verbose {
            crate::status!(
                "[runtime] pool {}/{}; mode {:?}; {} initialization thread limit",
                pool + 1,
                options.pools.len(),
                options.tuning.mode,
                options.init_threads.unwrap_or(0)
            );
        }
        let result = miner::run_control(current, control);
        monitor_stop.store(true, Ordering::Release);
        monitor
            .join()
            .map_err(|_| anyhow::anyhow!("runtime monitor panicked"))?;
        if stop.load(Ordering::Acquire) {
            return result;
        }
        if let Some(replacement) = reloaded.lock().unwrap_or_else(|p| p.into_inner()).take() {
            options = replacement;
            pool = 0;
            attempts = 0;
            crate::status!("[config] restarting workers with validated configuration");
            continue;
        }
        match result {
            Ok(()) => return Ok(()),
            Err(error)
                if is_pool_failure(&error)
                    && (options.pools.len() > 1
                        || error.to_string() == "user pool retries exhausted") =>
            {
                attempts += 1;
                if error.to_string().contains("user pool retries exhausted") {
                    attempts = options.pools[pool].retries;
                }
                crate::status!("[net] pool {} failed: {error:#}", pool + 1);
                if attempts >= options.pools[pool].retries {
                    pool = (pool + 1) % options.pools.len();
                    attempts = 0;
                    crate::status!("[net] switching to pool {}", pool + 1);
                }
                for _ in 0..options.pools[pool].retry_pause.saturating_mul(10) {
                    if stop.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn reload_options(cli: &Cli, bytes: &[u8], background: bool) -> Result<Options> {
    let options = Options::from_json(cli.clone(), bytes)?;
    anyhow::ensure!(
        options.background == background,
        "changing background mode requires restarting the process"
    );
    options.select_cores(&topology::discover()?, topology::cpu_quota())?;
    Ok(options)
}

fn is_pool_failure(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.starts_with("user pool retries exhausted")
        || message.starts_with("developer connection unavailable")
        || message == "developer connection dropped"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_configuration_writes_never_publish_partial_json() {
        let path =
            std::env::temp_dir().join(format!("rxminer-atomic-test-{}.json", std::process::id()));
        let old = br#"{"cpu":{"rx":[-1]}}"#;
        let new = br#"{"cpu":{"rx":[-1,-1]},"watch":false}"#;
        atomic_write(&path, old).unwrap();
        let stop = AtomicBool::new(false);
        thread::scope(|scope| {
            let reader = scope.spawn(|| {
                while !stop.load(Ordering::Acquire) {
                    let bytes = fs::read(&path).unwrap();
                    assert!(bytes == old || bytes == new);
                }
            });
            for _ in 0..20 {
                atomic_write(&path, new).unwrap();
                atomic_write(&path, old).unwrap();
            }
            stop.store(true, Ordering::Release);
            reader.join().unwrap();
        });
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(path).unwrap();
        assert!(!is_pool_failure(&anyhow::anyhow!("create RandomX VM")));
        assert!(is_pool_failure(&anyhow::anyhow!(
            "user pool retries exhausted"
        )));
    }
}
