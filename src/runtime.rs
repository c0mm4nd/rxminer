use crate::{
    config::{Cli, Options},
    miner, topology,
};
use anyhow::{Context, Result};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub fn install_signals(stop: Arc<AtomicBool>) -> Result<()> {
    #[cfg(unix)]
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, stop.clone())?;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{
            SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT,
        };
        static STOP: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
        unsafe extern "system" fn handle(event: u32) -> windows_sys::core::BOOL {
            if matches!(event, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
                if let Some(stop) = STOP.get() {
                    stop.store(true, Ordering::Release);
                }
                return 1;
            }
            0
        }
        STOP.set(stop)
            .map_err(|_| anyhow::anyhow!("console handler already installed"))?;
        // SAFETY: handler and STOP live for the process lifetime; the handler only stores an atomic flag.
        if unsafe { SetConsoleCtrlHandler(Some(handle), 1) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

// Called by main before signal registration or creation of any worker thread.
#[cfg(unix)]
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

#[cfg(not(unix))]
pub fn background() -> Result<bool> {
    anyhow::bail!("background mode is not implemented on this platform")
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
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // Rust 1.88 already handles Windows replacement while readers hold
        // the old file open, using FileRenameInfoEx when MoveFileEx fails.
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
    // Finite benchmarks must still report failure (including checksum errors).
    let recover = options.benchmark.is_none();
    let mut observed = cli.config.as_ref().and_then(|p| fs::read(p).ok());
    supervise(
        &stop,
        recover,
        || run_session(&mut options, &cli, &mut observed, stop.clone()),
        wait_for_retry,
    )
}

fn supervise(
    stop: &AtomicBool,
    recover: bool,
    mut attempt: impl FnMut() -> Result<()>,
    mut wait: impl FnMut(Duration, &AtomicBool),
) -> Result<()> {
    let mut delay = Duration::from_secs(5);
    while !stop.load(Ordering::Acquire) {
        let started = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(&mut attempt)).unwrap_or_else(|panic| {
            let message = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            Err(anyhow::anyhow!("mining coordinator panicked: {message}"))
        });
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let Err(error) = result else {
            // API stop, signal, or successful completion is intentional.
            return Ok(());
        };
        if !recover {
            return Err(error);
        }
        if started.elapsed() >= Duration::from_secs(60) {
            delay = Duration::from_secs(5);
        }
        crate::status!(
            "[runtime] {error:#}; rebuilding mining session in {}s",
            delay.as_secs()
        );
        wait(delay, stop);
        delay = (delay * 2).min(Duration::from_secs(60));
    }
    Ok(())
}

pub(crate) fn wait_for_retry(delay: Duration, stop: &AtomicBool) {
    let deadline = Instant::now() + delay;
    while !stop.load(Ordering::Acquire) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        thread::sleep(remaining.min(Duration::from_millis(100)));
    }
}

// Join the monitor even when the coordinator unwinds. Detached monitors from
// previous attempts must never reload configuration or stop a later session.
struct RuntimeMonitor {
    control: crate::control::Control,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RuntimeMonitor {
    fn finish(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("runtime monitor panicked"))?;
        }
        Ok(())
    }
}

impl Drop for RuntimeMonitor {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Release);
        let _ = self.finish();
    }
}

fn run_session(
    options: &mut Options,
    cli: &Cli,
    observed: &mut Option<Vec<u8>>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let mut pool = 0;
    let mut attempts = 0;
    let background = options.background;
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if options.watch {
            if let Some(bytes) = cli.config.as_ref().and_then(|p| fs::read(p).ok()) {
                if observed.as_ref() != Some(&bytes) {
                    *observed = Some(bytes.clone());
                    match reload_options(cli, &bytes, background) {
                        Ok(replacement) => {
                            *options = replacement;
                            pool = 0;
                            attempts = 0;
                        }
                        Err(error) => crate::status!("[config] reload rejected: {error:#}"),
                    }
                }
            }
        }
        crate::logging::configure(options)?;
        crate::randomx::set_thp(options.thp);
        crate::randomx::configure_jit(options.huge_pages_jit, options.prefetch_mode);
        let memory_pool = match options.memory_pool {
            Some(pages) => pages,
            None => options
                .select_cores(&topology::discover()?, topology::cpu_quota())?
                .len(),
        };
        crate::randomx::set_memory_pool(memory_pool);
        autosave(options, cli)?;
        if options.autosave && options.profile.is_none() {
            if let Some(bytes) = cli.config.as_ref().and_then(|p| fs::read(p).ok()) {
                *options = reload_options(cli, &bytes, background)?;
                *observed = Some(bytes);
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
                    let _panic = control.watch_thread("runtime monitor");
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
        let mut monitor = RuntimeMonitor {
            control: control.clone(),
            stop: monitor_stop,
            thread: Some(monitor),
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
        monitor.finish()?;
        if stop.load(Ordering::Acquire) {
            return result;
        }
        if let Some(replacement) = reloaded.lock().unwrap_or_else(|p| p.into_inner()).take() {
            *options = replacement;
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
        || message.starts_with("required pool connection unavailable")
        || message == "required pool connection dropped"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_rebuilds_after_errors_and_panics_with_bounded_backoff() {
        struct Released<'a>(&'a AtomicU64);
        impl Drop for Released<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let stop = AtomicBool::new(false);
        let released = AtomicU64::new(0);
        let mut attempts = 0;
        let mut delays = Vec::new();
        supervise(
            &stop,
            true,
            || {
                assert_eq!(released.load(Ordering::Relaxed), attempts);
                let _resource = Released(&released);
                attempts += 1;
                match attempts {
                    1 => anyhow::bail!("allocate dataset failed"),
                    2 => panic!("injected coordinator panic"),
                    3 => anyhow::bail!("pool jobs have inconsistent seeds for 120 seconds"),
                    4..=7 => anyhow::bail!("RandomX worker failed"),
                    _ => Ok(()),
                }
            },
            |delay, _| delays.push(delay.as_secs()),
        )
        .unwrap();
        assert_eq!(attempts, 8);
        assert_eq!(released.load(Ordering::Relaxed), 8);
        assert_eq!(delays, [5, 10, 20, 40, 60, 60, 60]);
    }

    #[test]
    fn explicit_stop_and_finite_failures_do_not_restart() {
        let stop = AtomicBool::new(false);
        supervise(
            &stop,
            true,
            || {
                stop.store(true, Ordering::Release);
                anyhow::bail!("error racing with shutdown")
            },
            |_, _| panic!("must not retry after stop"),
        )
        .unwrap();
        stop.store(false, Ordering::Release);
        assert!(supervise(
            &stop,
            false,
            || anyhow::bail!("benchmark checksum mismatch"),
            |_, _| panic!("must not retry a finite check"),
        )
        .is_err());
        supervise(
            &stop,
            true,
            || Ok(()),
            |_, _| panic!("API stop must complete"),
        )
        .unwrap();
        let started = Instant::now();
        thread::scope(|s| {
            s.spawn(|| {
                thread::sleep(Duration::from_millis(30));
                stop.store(true, Ordering::Release);
            });
            wait_for_retry(Duration::from_secs(60), &stop);
        });
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn coordinator_unwind_stops_and_joins_its_monitor() {
        let control = crate::control::Control::new(Arc::new(AtomicBool::new(false)));
        let finished = Arc::new(AtomicBool::new(false));
        let result = catch_unwind(AssertUnwindSafe(|| {
            let stop = Arc::new(AtomicBool::new(false));
            let monitor_stop = stop.clone();
            let worker_control = control.clone();
            let finished = finished.clone();
            let _monitor = RuntimeMonitor {
                control: control.clone(),
                stop,
                thread: Some(thread::spawn(move || {
                    while !monitor_stop.load(Ordering::Acquire) {
                        thread::yield_now();
                    }
                    assert!(worker_control.stopped());
                    finished.store(true, Ordering::Release);
                })),
            };
            panic!("injected coordinator panic");
        }));
        assert!(result.is_err());
        assert!(control.stopped());
        assert!(finished.load(Ordering::Acquire));
    }

    #[test]
    fn atomic_configuration_writes_never_publish_partial_json() {
        let path =
            std::env::temp_dir().join(format!("rxminer-atomic-test-{}.json", std::process::id()));
        let old = br#"{"cpu":{"rx":[-1]}}"#;
        let new = br#"{"cpu":{"rx":[-1,-1]},"watch":false}"#;
        atomic_write(&path, old).unwrap();
        let stop = AtomicBool::new(false);
        thread::scope(|scope| {
            struct StopReader<'a>(&'a AtomicBool);
            impl Drop for StopReader<'_> {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                }
            }
            // Stop the reader even if a filesystem operation fails and the
            // writer unwinds; otherwise scope teardown would wait forever.
            let stop_reader = StopReader(&stop);
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
            drop(stop_reader);
            reader.join().unwrap();
        });
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_file(path).unwrap();
        assert!(!is_pool_failure(&anyhow::anyhow!("create RandomX VM")));
        assert!(is_pool_failure(&anyhow::anyhow!(
            "user pool retries exhausted"
        )));
    }
}
