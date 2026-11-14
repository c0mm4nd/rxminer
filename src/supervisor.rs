//! Foreground process supervision for failures that cannot unwind in Rust.
use anyhow::{Context, Result};
use std::{
    io::{self, Read},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const WORKER_ENV: &str = "RXMINER_INTERNAL_WORKER";

pub fn is_worker() -> bool {
    std::env::var_os(WORKER_ENV).is_some_and(|value| value == "1")
}

// The parent owns the write end of this pipe. Closing it requests shutdown,
// including when the parent itself disappears. No shell or external service
// is required. This one process-lifetime reader exits with the worker process.
pub fn watch_parent(stop: Arc<AtomicBool>) -> Result<()> {
    thread::Builder::new()
        .name("parent-control".into())
        .spawn(move || {
            let mut input = io::stdin().lock();
            loop {
                match input.read(&mut [0u8; 1]) {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    _ => break,
                }
            }
            stop.store(true, Ordering::Release);
        })
        .context("start parent control reader")?;
    Ok(())
}

struct Worker(Child);

impl Worker {
    fn shutdown(&mut self) -> Result<()> {
        // Closing stdin wakes the worker's control reader on Unix and Windows.
        drop(self.0.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.0.try_wait()?.is_none() {
            if Instant::now() >= deadline {
                self.0.kill().context("stop unresponsive mining process")?;
                self.0.wait().context("reap mining process")?;
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

pub fn run(stop: Arc<AtomicBool>) -> Result<()> {
    let mut delay = Duration::from_secs(5);
    while !stop.load(Ordering::Acquire) {
        let started = Instant::now();
        let result = run_worker(&stop);
        if stop.load(Ordering::Acquire) {
            return result;
        }
        let Err(error) = result else {
            // A clean worker exit includes an authenticated API stop.
            return Ok(());
        };
        if started.elapsed() >= Duration::from_secs(60) {
            delay = Duration::from_secs(5);
        }
        crate::status!(
            "[supervisor] {error:#}; restarting mining process in {}s",
            delay.as_secs()
        );
        crate::runtime::wait_for_retry(delay, &stop);
        delay = (delay * 2).min(Duration::from_secs(60));
    }
    Ok(())
}

fn run_worker(stop: &AtomicBool) -> Result<()> {
    let child = Command::new(std::env::current_exe().context("locate miner executable")?)
        .args(std::env::args_os().skip(1))
        .env(WORKER_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("start mining process")?;
    let mut worker = Worker(child);
    crate::status!("[supervisor] started mining process {}", worker.0.id());
    loop {
        if stop.load(Ordering::Acquire) {
            return worker.shutdown();
        }
        if let Some(status) = worker.0.try_wait().context("inspect mining process")? {
            anyhow::ensure!(status.success(), "mining process exited: {status}");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
}
