//! Portable native consensus vectors, linked to the archive built by Cargo.
mod support;
use anyhow::{ensure, Context, Result};
use std::{fs::OpenOptions, io::Write, path::Path, process::Command};
use support::{root, run, TempDir};

#[test]
#[ignore = "runs the native vector suite, including interpreter and JIT paths"]
fn native_vectors() -> Result<()> {
    let temp = TempDir::new("native-vectors")?;
    let library = Path::new(env!("OUT_DIR")).join(if cfg!(target_env = "msvc") {
        "randomx.lib"
    } else {
        "librandomx.a"
    });
    ensure!(library.is_file(), "Cargo-built RandomX archive missing");
    let version = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("-vV")
        .output()?;
    ensure!(version.status.success(), "rustc -vV failed");
    let text = String::from_utf8(version.stdout)?;
    let host = text
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .context("rustc host")?;
    let compiler = cc::Build::new()
        .cpp(true)
        .host(host)
        .target(host)
        .opt_level(2)
        .debug(false)
        .cargo_metadata(false)
        .out_dir(&temp.0)
        .get_compiler();
    for source in [
        "tests",
        "program_generation",
        "jit_normalization",
        "blake_fixed",
        "blake_rounds",
        "aes_pipeline",
    ] {
        let binary = temp
            .0
            .join(format!("{source}{}", std::env::consts::EXE_SUFFIX));
        let mut command = compiler.to_command();
        // Keep compiler side products out of the checkout on Windows.
        command.current_dir(&temp.0);
        if compiler.is_like_msvc() {
            command.args(["/std:c++14", "/O2", "/EHsc", "/D__AES__", "/I"]);
        } else {
            command.args(["-std=c++11", "-O2", "-DHAVE_POSIX_MEMALIGN", "-I"]);
        }
        command.arg(root().join("src/crypto/randomx"));
        if !compiler.is_like_msvc() {
            if cfg!(target_arch = "x86_64") {
                command.arg("-maes");
            }
            if cfg!(target_arch = "aarch64") {
                command.arg("-march=armv8-a+crypto");
            }
        }
        command.arg(root().join(format!("tests/randomx/{source}.cpp")));
        if matches!(source, "blake_rounds" | "aes_pipeline") {
            let aes = source == "aes_pipeline";
            let mut reference = cc::Build::new();
            reference
                .host(host)
                .target(host)
                .cpp(aes)
                .std(if aes { "c++11" } else { "c11" })
                .opt_level(3)
                .debug(false)
                .pic(true)
                .define("NDEBUG", None)
                .warnings(false)
                .cargo_metadata(false)
                .out_dir(&temp.0)
                .include(root().join("src/crypto/randomx"))
                .file(root().join(if aes {
                    "tests/randomx/aes_legacy.cpp"
                } else {
                    "tests/randomx/blake_compact.c"
                }));
            if compiler.is_like_msvc() && cfg!(target_arch = "x86_64") {
                reference.define("__AES__", None);
            }
            if !compiler.is_like_msvc() {
                reference.flag_if_supported("-fno-semantic-interposition");
                if cfg!(target_arch = "x86_64") {
                    reference.flag("-maes");
                }
                if cfg!(target_arch = "aarch64") {
                    reference.flag("-march=armv8-a+crypto");
                }
                if std::env::var("RXMINER_ARCH").as_deref() == Ok("native") {
                    reference.flag("-march=native");
                }
            }
            command.args(reference.compile_intermediates());
        }
        command.arg(&library);
        if compiler.is_like_msvc() {
            command
                .arg("advapi32.lib")
                .arg(format!("/Fe{}", binary.display()));
        } else {
            command.args(["-pthread", "-lm", "-o"]).arg(&binary);
        }
        run(
            &mut command,
            temp.0.join(format!("{source}-compile.log")),
            300,
        )?;
        let mut execute = Command::new(binary);
        if matches!(source, "blake_rounds" | "aes_pipeline")
            && std::env::var("RXMINER_SHORT_BENCH").as_deref() == Ok("1")
        {
            execute.arg("--sample");
        }
        let output = run(
            &mut execute,
            temp.0.join(format!("{source}.log")),
            if source == "tests" { 900 } else { 15 },
        )?;
        println!("{output}");
        if source == "blake_rounds" {
            write_short_summary(&output)?;
        }
        if source == "aes_pipeline" {
            write_aes_summary(&output)?;
        }
    }
    Ok(())
}

fn write_short_summary(output: &str) -> Result<()> {
    let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") else {
        return Ok(());
    };
    let mut rows = String::new();
    for line in output
        .lines()
        .filter_map(|line| line.strip_prefix("RXM_SHORT_SAMPLE "))
    {
        let row: serde_json::Value = serde_json::from_str(line)?;
        let compact = row["compact_ns"].as_f64().context("compact timing")?;
        let optimized = row["optimized_ns"].as_f64().context("optimized timing")?;
        ensure!(compact > 0.0 && optimized > 0.0, "invalid timing sample");
        rows.push_str(&format!(
            "| {} → {} B | {} | {compact:.1} | {optimized:.1} | {:.3}× |\n",
            row["input"],
            row["output"],
            row["pairs"],
            compact / optimized
        ));
    }
    if !rows.is_empty() {
        let mut summary = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(summary, "### BLAKE2b preparation: short sample\n\nSame process, same C compiler and optimization level; alternating compact/reference and current/archive runs. Median nanoseconds per operation; ratio = compact / current. Sampling budget: 500 ms, checked between fixed batches. Informational only: shared-runner timings are not mining hashrate and do not gate correctness.\n\n| Input → digest | Sample pairs | Compact ns/op | Current ns/op | Ratio |\n| --- | ---: | ---: | ---: | ---: |\n{rows}")?;
    }
    Ok(())
}

fn write_aes_summary(output: &str) -> Result<()> {
    let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") else {
        return Ok(());
    };
    let mut rows = String::new();
    for line in output
        .lines()
        .filter_map(|line| line.strip_prefix("RXM_AES_SAMPLE "))
    {
        let row: serde_json::Value = serde_json::from_str(line)?;
        let legacy = row["legacy_ns"].as_f64().context("legacy AES timing")?;
        let current = row["current_ns"].as_f64().context("current AES timing")?;
        ensure!(legacy > 0.0 && current > 0.0, "invalid AES timing");
        rows.push_str(&format!(
            "| {} | {} | {:.3} | {:.3} | {:.3}× |\n",
            row["mode"].as_str().context("AES mode")?,
            row["pairs"],
            legacy / 1e6,
            current / 1e6,
            legacy / current
        ));
    }
    if !rows.is_empty() {
        let mut summary = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(summary, "### AES scratchpad hash/refill: short sample\n\nOne 2 MiB scratchpad per call; same compiler and optimization level, alternating old/current paths. Complete scratchpad, hash and generator state compared outside timing. Median milliseconds per call; ratio = legacy / current. Sampling budget: 500 ms, checked between pairs. BLAKE2b and AES together budget 1 second plus the final bounded batch. These are kernel timings, not mining hashrate; performance does not gate correctness.\n\n| AES mode | Sample pairs | Legacy ms | Current ms | Ratio |\n| --- | ---: | ---: | ---: | ---: |\n{rows}")?;
    }
    Ok(())
}
