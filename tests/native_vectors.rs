#![cfg(feature = "randomx-rust")]
//! Optional oracle executables built separately with CMake; Cargo compiles only Rust.
#[path = "support/randomx_core.rs"]
mod randomx_core;
#[path = "support/randomx_vm.rs"]
mod randomx_vm;
mod support;
use anyhow::{ensure, Context, Result};
use std::{path::PathBuf, process::Command};
use support::{run, TempDir};

#[test]
#[ignore = "runs the native vector suite, including interpreter and JIT paths"]
fn native_vectors() -> Result<()> {
    let temp = TempDir::new("native-vectors")?;
    let directory = PathBuf::from(
        std::env::var_os("RXMINER_ORACLE_DIR")
            .context("set RXMINER_ORACLE_DIR to the separately built CMake oracle directory")?,
    );
    for source in [
        "tests",
        "program_generation",
        "jit_normalization",
        "blake_fixed",
        "blake_rounds",
        "aes_pipeline",
        "interpreter_dispatch",
        "rust_core_oracle",
        "rust_vm_oracle",
        "jit_optimizations",
        "jit_rounding",
        "jit_swaps",
        "jit_memory",
    ] {
        let name = if source == "tests" {
            "randomx-tests"
        } else {
            source
        };
        let binary = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        ensure!(
            binary.is_file(),
            "oracle executable missing: {}",
            binary.display()
        );
        let mut execute = Command::new(binary);
        if matches!(source, "rust_core_oracle" | "rust_vm_oracle") {
            execute.arg(temp.0.join(format!("{source}.bin")));
        }
        let output = run(
            &mut execute,
            temp.0.join(format!("{source}.log")),
            if source == "tests" {
                900
            } else if source == "rust_vm_oracle" {
                120
            } else if source.starts_with("jit_") {
                180
            } else {
                15
            },
        )?;
        println!("{output}");
        if source == "jit_optimizations" {
            for args in [
                vec!["--normalized"],
                vec!["--compact"],
                vec!["--compact", "--normalized"],
            ] {
                let mut execute = Command::new(
                    directory.join(format!("{source}{}", std::env::consts::EXE_SUFFIX)),
                );
                execute.args(args);
                println!("{}", run(&mut execute, temp.0.join("jit-extra.log"), 180)?);
            }
        }
        if source == "rust_core_oracle" {
            randomx_core::compare(&temp.0.join("rust_core_oracle.bin"))?;
        }
        if source == "rust_vm_oracle" {
            randomx_vm::compare(&temp.0.join("rust_vm_oracle.bin"))?;
        }
    }
    Ok(())
}
