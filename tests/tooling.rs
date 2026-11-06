#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
//! Developer checks run explicitly; ordinary `cargo test` does not rebuild packages.
mod support;

use anyhow::{ensure, Context, Result};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use support::{root, run, TempDir};

#[test]
#[ignore = "compiles and runs the native consensus and differential suites"]
fn native_regressions() -> Result<()> {
    let temp = TempDir::new("native-tests")?;
    let library = Path::new(env!("OUT_DIR")).join("librandomx.a");
    ensure!(library.is_file(), "Cargo-built RandomX archive missing");
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = Command::new(rustc).arg("-vV").output()?;
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
    for name in [
        "tests",
        "jit_optimizations",
        "program_generation",
        "blake_fixed",
    ] {
        let binary = temp.0.join(name);
        let mut command = compiler.to_command();
        command
            .args(["-std=c++11", "-O2", "-maes", "-DHAVE_POSIX_MEMALIGN", "-I"])
            .arg(root().join("src/crypto/randomx"))
            .arg(root().join(format!("tests/randomx/{name}.cpp")))
            .arg(&library)
            .args(["-pthread", "-lm", "-o"])
            .arg(&binary);
        run(&mut command, temp.0.join("compile.log"), 120)?;
        let output = run(&mut Command::new(&binary), temp.0.join("native.log"), 180)?;
        println!("{output}");
        if name == "jit_optimizations" {
            println!(
                "{}",
                run(
                    Command::new(&binary).arg("--normalized"),
                    temp.0.join("normalized.log"),
                    180
                )?
            );
        }
    }
    Ok(())
}

fn guarded(command: &mut Command, guard: &Path, guard_log: &Path) -> Result<()> {
    let mut paths = vec![guard.to_owned()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    command
        .env("PATH", std::env::join_paths(paths)?)
        .env("RXMINER_BUILD_GUARD_LOG", guard_log)
        .env("CMAKE", guard.join("cmake"))
        .env("MAKE", guard.join("make"))
        .env_remove("RXMINER_ARCH")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    Ok(())
}

fn check_installed(binary: &Path, temp: &Path) -> Result<()> {
    for flag in ["--version", "--help"] {
        run(
            Command::new(binary).arg(flag).current_dir(temp),
            temp.join("smoke.log"),
            10,
        )?;
    }
    for flag in ["--no-huge-pages", "--no-thp"] {
        let mut command = support::miner_command(binary)?;
        command.current_dir(temp).args([
            "--benchmark-seconds",
            "1",
            "--threads",
            "1",
            "--randomx-no-numa",
            flag,
        ]);
        let output = run(&mut command, temp.join("hashing.log"), 180)?;
        let hashes: u64 = output
            .split("benchmark finished: ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .context("missing benchmark count")?
            .parse()?;
        ensure!(hashes > 0, "installed miner produced no hashes");
        if flag == "--no-thp" {
            ensure!(
                output.contains("explicit 2 MiB pages:"),
                "missing page coverage"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "fresh package, local and Git installations; needs compilers, Git and 4 GiB free RAM"]
fn cargo_install_without_external_build_tools() -> Result<()> {
    let temp = TempDir::new("cargo-install")?;
    let guard = temp.0.join("blocked-tools");
    fs::create_dir(&guard)?;
    let guard_log = temp.0.join("unexpected-tool.log");
    for tool in [
        "cmake", "cmake3", "ccmake", "make", "gmake", "ninja", "python", "python3",
    ] {
        let path = guard.join(tool);
        fs::write(
            &path,
            "#!/bin/sh\nprintf '%s\\n' \"$0\" >> \"$RXMINER_BUILD_GUARD_LOG\"\nexit 97\n",
        )?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut package = Command::new(&cargo);
    package
        .current_dir(root())
        .args([
            "package",
            "--locked",
            "--allow-dirty",
            "-j",
            "2",
            "--target-dir",
        ])
        .arg(temp.0.join("package-target"));
    guarded(&mut package, &guard, &guard_log)?;
    run(&mut package, temp.0.join("package.log"), 900)?;

    // Reproduce the packaged file set without copying .git, build artifacts,
    // generated certificates or environment-specific reports into the Git fixture.
    let mut list = Command::new(&cargo);
    list.current_dir(root())
        .args(["package", "--list", "--locked", "--allow-dirty"]);
    guarded(&mut list, &guard, &guard_log)?;
    let listed = list.output()?;
    ensure!(
        listed.status.success(),
        "cargo package --list failed: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let checkout = temp.0.join("git-source");
    fs::create_dir(&checkout)?;
    for name in String::from_utf8(listed.stdout)?.lines() {
        if matches!(name, "Cargo.toml.orig" | ".cargo_vcs_info.json") {
            continue;
        }
        let relative = Path::new(name);
        ensure!(
            relative
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
            "invalid package path"
        );
        let destination = checkout.join(relative);
        fs::create_dir_all(destination.parent().context("package parent")?)?;
        fs::copy(root().join(relative), destination)?;
    }
    for args in [
        vec!["init", "--quiet", "--initial-branch=main"],
        vec!["add", "--all"],
        vec![
            "-c",
            "user.name=Command",
            "-c",
            "user.email=me@c0mm4nd.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "Installation test snapshot",
        ],
    ] {
        run(
            Command::new("git").args(args).current_dir(&checkout),
            temp.0.join("git.log"),
            30,
        )?;
    }
    for mode in ["local", "git"] {
        let install = temp.0.join(format!("{mode}-installed"));
        let mut command = Command::new(&cargo);
        command.current_dir(&temp.0).arg("install");
        if mode == "local" {
            command.arg("--path").arg(root());
        } else {
            command.arg("--git").arg(
                url::Url::from_directory_path(&checkout)
                    .map_err(|()| anyhow::anyhow!("Git fixture URL"))?
                    .as_str(),
            );
        }
        command
            .args(["--locked", "--bin", "rxminer", "-j", "2", "--root"])
            .arg(&install)
            .arg("--target-dir")
            .arg(temp.0.join(format!("{mode}-target")));
        guarded(&mut command, &guard, &guard_log)?;
        run(
            &mut command,
            temp.0.join(format!("{mode}-install.log")),
            900,
        )?;
        check_installed(&install.join("bin/rxminer"), &temp.0)?;
    }
    ensure!(
        !guard_log.exists(),
        "installation invoked a blocked tool: {}",
        fs::read_to_string(&guard_log).unwrap_or_default()
    );
    println!("PASS: package, local/Git install and hashing without Python, CMake, Make or Ninja");
    Ok(())
}
