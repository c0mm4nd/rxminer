#![cfg(all(feature = "randomx-rust", target_os = "linux", target_arch = "x86_64"))]
//! Developer checks run explicitly; ordinary `cargo test` does not rebuild packages.
mod support;

use anyhow::{ensure, Context, Result};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use support::{root, run, TempDir};

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
        .env("CXX", guard.join("c++"))
        .env("CXX_x86_64_unknown_linux_gnu", guard.join("c++"))
        // A host with OpenSSL installed must not hide a TLS build dependency.
        .env("OPENSSL_DIR", guard.join("no-openssl"))
        .env("OPENSSL_LIB_DIR", guard.join("no-openssl/lib"))
        .env("OPENSSL_INCLUDE_DIR", guard.join("no-openssl/include"))
        .env("OPENSSL_NO_VENDOR", "1")
        .env_remove("RXMINER_ARCH")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    Ok(())
}

fn check_installed(binary: &Path, temp: &Path) -> Result<()> {
    let libraries = run(
        Command::new("ldd").arg(binary),
        temp.join("dependencies.log"),
        10,
    )?;
    ensure!(
        !libraries.contains("libssl.")
            && !libraries.contains("libcrypto.")
            && !libraries.contains("libstdc++"),
        "installed binary still links OpenSSL/C++: {libraries}"
    );
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
            "--randomx-mode",
            "light",
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
        "cmake",
        "cmake3",
        "ccmake",
        "make",
        "gmake",
        "ninja",
        "python",
        "python3",
        "openssl",
        "pkg-config",
        "c++",
        "g++",
        "clang++",
    ] {
        let path = guard.join(tool);
        fs::write(
            &path,
            "#!/bin/sh\nprintf '%s\\n' \"$0\" >> \"$RXMINER_BUILD_GUARD_LOG\"\nexit 97\n",
        )?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    for package_name in ["randomx-rust", "rx-platform"] {
        let mut package = Command::new(&cargo);
        package
            .current_dir(root())
            .args([
                "package",
                "-p",
                package_name,
                "--locked",
                "--allow-dirty",
                "-j",
                "2",
                "--target-dir",
            ])
            .arg(temp.0.join("package-target"));
        guarded(&mut package, &guard, &guard_log)?;
        run(
            &mut package,
            temp.0.join(format!("{package_name}-package.log")),
            900,
        )?;
    }

    // Backend and platform crates must be published before a registry package of the miner
    // can resolve them. Verify the standalone Rust crates above, and reproduce
    // the complete local/Git workspace below without .git or build artifacts.
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
    fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let target = destination.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_tree(&entry.path(), &target)?;
            } else {
                fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
    copy_tree(&root().join("crates"), &checkout.join("crates"))?;
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
    println!("PASS: package, local/Git install and hashing without C++, Python, CMake, Make, Ninja or OpenSSL; ring still uses a C compiler");
    Ok(())
}
