//! Real CLI smoke checks shared by every OS/CPU runner.
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Miner(Child);
impl Drop for Miner {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[test]
fn cli_version_and_configuration() {
    let binary = env!("CARGO_BIN_EXE_rxminer");
    for args in [
        vec!["--version"],
        vec!["--help"],
        vec![
            "--benchmark-seconds",
            "1",
            "--randomx-mode",
            "light",
            "--dry-run",
        ],
    ] {
        let output = Command::new(binary).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "runs light and full-memory hashing; needs 4 GiB of free RAM"]
fn light_and_full_memory_mining() {
    for mode in ["light", "fast"] {
        let log: PathBuf = std::env::temp_dir().join(format!(
            "rxminer-portable-{}-{mode}.log",
            std::process::id()
        ));
        let file = fs::File::create(&log).unwrap();
        let mut miner = Miner(
            Command::new(env!("CARGO_BIN_EXE_rxminer"))
                .args([
                    "--benchmark-seconds",
                    "1",
                    "--randomx-mode",
                    mode,
                    "--threads",
                    "1",
                    "--randomx-init",
                    "2",
                    "--randomx-no-numa",
                    "--no-huge-pages",
                    "--no-proxy",
                ])
                .stdin(Stdio::null())
                .stdout(file.try_clone().unwrap())
                .stderr(file)
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now()
            + Duration::from_secs(if cfg!(target_arch = "riscv64") {
                900
            } else {
                180
            });
        let status = loop {
            if let Some(status) = miner.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "{mode} mining timed out: {}",
                fs::read_to_string(&log).unwrap_or_default()
            );
            thread::sleep(Duration::from_millis(100));
        };
        let output = fs::read_to_string(&log).unwrap();
        assert!(status.success(), "{mode} failed: {output}");
        let count: u64 = output
            .split("benchmark finished: ")
            .nth(1)
            .and_then(|line| line.split_whitespace().next())
            .expect(&output)
            .parse()
            .unwrap();
        assert!(count > 0, "{mode} produced no hashes");
        assert!(
            !output.contains("switching to light mode"),
            "fast mode silently fell back: {output}"
        );
        fs::remove_file(log).unwrap();
    }
}
