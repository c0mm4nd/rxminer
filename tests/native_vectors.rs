//! Portable native consensus vectors, linked to the archive built by Cargo.
mod support;
use anyhow::{ensure, Context, Result};
use std::{path::Path, process::Command};
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
    for source in ["tests", "program_generation", "jit_normalization"] {
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
        command
            .arg(root().join(format!("tests/randomx/{source}.cpp")))
            .arg(&library);
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
        println!(
            "{}",
            run(
                &mut Command::new(binary),
                temp.0.join(format!("{source}.log")),
                900
            )?
        );
    }
    Ok(())
}
