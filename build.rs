use std::{env, path::PathBuf};

const C_SOURCES: &[&str] = &[
    "argon2_ref.c",
    "argon2_core.c",
    "blake2/blake2b.c",
    "reciprocal.c",
    "virtual_memory.c",
    "jit_compiler_x86_static.S",
];
const CPP_SOURCES: &[&str] = &[
    "aes_hash.cpp",
    "allocator.cpp",
    "blake2_generator.cpp",
    "bytecode_machine.cpp",
    "cpu.cpp",
    "dataset.cpp",
    "instruction.cpp",
    "instructions_portable.cpp",
    "jit_compiler_x86.cpp",
    "randomx.cpp",
    "soft_aes.cpp",
    "superscalar.cpp",
    "virtual_machine.cpp",
    "vm_compiled.cpp",
    "vm_compiled_light.cpp",
    "vm_interpreted.cpp",
    "vm_interpreted_light.cpp",
];

fn main() {
    assert_eq!(
        env::var("CARGO_CFG_TARGET_OS").unwrap(),
        "linux",
        "Linux is required"
    );
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ARCH").unwrap(),
        "x86_64",
        "x86-64 is required"
    );
    assert_eq!(
        env::var("HOST").unwrap(),
        env::var("TARGET").unwrap(),
        "cross compilation is not configured"
    );
    println!("cargo:rerun-if-changed=src/crypto/randomx");
    println!("cargo:rerun-if-env-changed=RXMINER_ARCH");
    let source =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("src/crypto/randomx");
    let arch = env::var("RXMINER_ARCH").unwrap_or_else(|_| "default".into());
    assert!(
        matches!(arch.as_str(), "default" | "native"),
        "RXMINER_ARCH must be default or native"
    );

    // Compile directly through Cargo/cc: no CMake, Make, generated project or
    // source-path-bound configuration cache is needed. Keep RandomX optimized
    // in debug builds too, as with the previous native Release configuration.
    let mut common = cc::Build::new();
    common
        .include(&source)
        .opt_level(3)
        .debug(false)
        .pic(true)
        .define("NDEBUG", None)
        .define("HAVE_POSIX_MEMALIGN", None)
        .warnings(false)
        .flag("-maes")
        // RandomX is linked statically into the executable. Its private native
        // calls cannot be interposed; let the compiler inline those calls.
        .flag_if_supported("-fno-semantic-interposition");
    if arch == "native" {
        common.flag("-march=native");
    }
    let mut c = common.clone();
    c.std("gnu11")
        .files(C_SOURCES.iter().map(|file| source.join(file)));
    let mut objects = c.compile_intermediates();

    // ISA-specific Argon2 kernels must stay in separate translation units.
    // The portable binary selects them using RandomX's runtime CPU checks.
    for (file, flag) in [("argon2_ssse3.c", "-mssse3"), ("argon2_avx2.c", "-mavx2")] {
        objects.extend(
            common
                .clone()
                .std("gnu11")
                .file(source.join(file))
                .flag(flag)
                .compile_intermediates(),
        );
    }

    let vaes = env::var_os("CARGO_FEATURE_VAES").is_some();
    if vaes {
        objects.extend(
            common
                .clone()
                .cpp(true)
                .std("c++11")
                .define("RANDOMX_HAVE_VAES", "1")
                .file(source.join("aes_hash_vaes.cpp"))
                .flag("-mavx2")
                .flag("-mvaes")
                .compile_intermediates(),
        );
    }
    let mut cpp = common;
    cpp.cpp(true)
        .std("c++11")
        .cpp_link_stdlib("stdc++")
        .files(CPP_SOURCES.iter().map(|file| source.join(file)))
        .objects(objects);
    if vaes {
        cpp.define("RANDOMX_HAVE_VAES", "1");
    }
    // One archive contains all C, C++ and assembly objects, avoiding cyclic
    // static-library link ordering between the JIT, VM and Argon2 helpers.
    cpp.compile("randomx");
    println!("cargo:rustc-link-lib=pthread");
    println!("cargo:rustc-link-lib=m");
}
