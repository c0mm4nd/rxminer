// Each integration-test crate uses a different subset of these helpers.
#![allow(dead_code)]

use anyhow::{bail, ensure, Context, Result};
use std::{
    fs::{self, File},
    net::TcpListener,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub struct TempDir(pub PathBuf);
impl TempDir {
    pub fn new(label: &str) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "rxminer-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn wait_until<T>(seconds: u64, mut predicate: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(value) = predicate()? {
            return Ok(value);
        }
        ensure!(
            Instant::now() < deadline,
            "test condition timed out after {seconds}s"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

pub fn port() -> Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

pub fn test_cpus() -> Result<Vec<usize>> {
    Ok(rxminer::topology::discover()?
        .into_iter()
        .filter(|core| !core.smt)
        .take(8)
        .map(|core| core.cpu)
        .collect())
}

pub fn miner_command(binary: &Path) -> Result<Command> {
    let mut command = Command::new("taskset");
    let cpus = test_cpus()?
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",");
    command.args(["-c", &cpus]).arg(binary).current_dir(root());
    Ok(command)
}

pub struct Process {
    child: Child,
    log: PathBuf,
}
impl Process {
    pub fn spawn(command: &mut Command, log: impl AsRef<Path>) -> Result<Self> {
        let log = log.as_ref().to_owned();
        let file = File::create(&log)?;
        let child = command
            .stdin(Stdio::null())
            .stdout(file.try_clone()?)
            .stderr(file)
            .process_group(0)
            .spawn()
            .with_context(|| format!("spawn {}", command.get_program().to_string_lossy()))?;
        Ok(Self { child, log })
    }

    pub fn output(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    pub fn ensure_running(&mut self) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            bail!("child exited early ({status}):\n{}", self.output());
        }
        Ok(())
    }

    pub fn signal(&mut self, signal: libc::c_int) -> Result<()> {
        if self.child.try_wait()?.is_none() {
            // SAFETY: this is the process group of our unreaped child, created by Command.
            let result = unsafe { libc::kill(-(self.child.id() as i32), signal) };
            if result != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(())
    }

    pub fn wait(&mut self, seconds: u64) -> Result<ExitStatus> {
        wait_until(seconds, || Ok(self.child.try_wait()?))
            .with_context(|| format!("child log:\n{}", self.output()))
    }

    pub fn success(&mut self, seconds: u64) -> Result<()> {
        let status = self.wait(seconds)?;
        ensure!(
            status.success(),
            "child failed ({status}):\n{}",
            self.output()
        );
        Ok(())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let running = self.child.try_wait().ok().flatten().is_none();
        if running || thread::panicking() {
            let output = self.output();
            let lines: Vec<_> = output.lines().collect();
            eprintln!(
                "Test log {}:",
                self.log.file_name().unwrap().to_string_lossy()
            );
            if lines.len() > 80 {
                eprintln!(
                    "{}\n...\n{}",
                    lines[..40].join("\n"),
                    lines[lines.len() - 40..].join("\n")
                );
            } else {
                eprintln!("{output}");
            }
        }
        if running {
            let _ = self.signal(libc::SIGINT);
            if self.wait(5).is_err() {
                let _ = self.signal(libc::SIGKILL);
                let _ = self.child.wait();
            }
        }
    }
}

pub fn run(command: &mut Command, log: impl AsRef<Path>, seconds: u64) -> Result<String> {
    // Compiler commands may contain a full environment snapshot. Log only
    // the executable and arguments, never environment values or credentials.
    eprintln!(
        "Running {:?} {:?}",
        command.get_program(),
        command.get_args().collect::<Vec<_>>()
    );
    let mut child = Process::spawn(command, log)?;
    child.success(seconds)?;
    Ok(child.output())
}

pub fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
