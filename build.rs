use std::{env, fs, path::PathBuf};

const C_SOURCES: &[&str] = &[
    "argon2_ref.c",
    "argon2_core.c",
    "blake2/blake2b.c",
    "reciprocal.c",
    "virtual_memory.c",
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
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let cpu = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let msvc = env::var("CARGO_CFG_TARGET_ENV").unwrap() == "msvc";
    println!("cargo:rerun-if-changed=src/crypto/randomx");
    println!("cargo:rerun-if-env-changed=RXMINER_ARCH");
    let source =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("src/crypto/randomx");
    let arch = env::var("RXMINER_ARCH").unwrap_or_else(|_| "default".into());
    assert!(
        matches!(arch.as_str(), "default" | "native"),
        "RXMINER_ARCH must be default or native"
    );
    if arch == "native" {
        assert_eq!(
            env::var("HOST").unwrap(),
            env::var("TARGET").unwrap(),
            "native CPU tuning cannot be used while cross-compiling"
        );
    }
    let mut common = cc::Build::new();
    common
        .include(&source)
        .opt_level(3)
        .debug(false)
        .pic(true)
        .define("NDEBUG", None)
        .warnings(false);
    if os != "windows" {
        common.define("HAVE_POSIX_MEMALIGN", None);
    }
    if !msvc {
        common.flag_if_supported("-fno-semantic-interposition");
        if cpu == "x86_64" {
            common.flag("-maes");
        }
        if cpu == "aarch64" {
            common.flag("-march=armv8-a+crypto");
        }
        if arch == "native" {
            common.flag("-march=native");
        }
    } else {
        // MSVC exposes AES intrinsics without a separate /arch:AES option.
        if cpu == "x86_64" {
            common.define("__AES__", None);
        }
    }
    let mut c = common.clone();
    c.std("c11")
        .files(C_SOURCES.iter().map(|file| source.join(file)));
    let mut objects = c.compile_intermediates();
    for (file, flag) in [("argon2_ssse3.c", "-mssse3"), ("argon2_avx2.c", "-mavx2")] {
        let mut kernel = common.clone();
        kernel.std("c11").file(source.join(file));
        if cpu == "x86_64" {
            if msvc {
                if file.contains("avx2") {
                    kernel.flag("/arch:AVX2");
                }
            } else {
                kernel.flag(flag);
            }
        }
        objects.extend(kernel.compile_intermediates());
    }
    let mut cpp = common.clone();
    let vaes = env::var_os("CARGO_FEATURE_VAES").is_some() && cpu == "x86_64" && !msvc;
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
        cpp.define("RANDOMX_HAVE_VAES", "1");
    }
    match cpu.as_str() {
        "x86_64" if os == "windows" => {
            cpp.file(source.join("jit_compiler_x86_windows.cpp"));
            let mut assembly = cc::Build::new();
            assembly.include(&source).opt_level(3).debug(false);
            if msvc {
                // Generate MASM constants in OUT_DIR directly from the same header.
                // No PowerShell converter or native build-system generator is needed.
                let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
                fs::create_dir_all(out.join("asm")).unwrap();
                let header = fs::read_to_string(source.join("configuration.h")).unwrap();
                let constants = header
                    .lines()
                    .filter_map(|line| {
                        let mut fields = line.split_whitespace();
                        if fields.next()? != "#define" {
                            return None;
                        }
                        let name = fields.next()?;
                        let value = fields.next()?;
                        value
                            .parse::<u64>()
                            .ok()
                            .map(|_| format!("{name} EQU {value}\n"))
                    })
                    .collect::<String>();
                fs::write(out.join("asm/configuration.asm"), constants).unwrap();
                assembly
                    .includes([&out, &source])
                    .file(source.join("jit_compiler_x86_windows_static.asm"));
            } else {
                assembly.file(source.join("jit_compiler_x86_windows_static.S"));
            }
            objects.extend(assembly.compile_intermediates());
        }
        "x86_64" => {
            cpp.file(source.join("jit_compiler_x86.cpp"));
            objects.extend(
                common
                    .clone()
                    .file(source.join("jit_compiler_x86_static.S"))
                    .compile_intermediates(),
            );
        }
        "aarch64" => {
            cpp.file(source.join("jit_compiler_a64.cpp"));
            objects.extend(
                common
                    .clone()
                    .file(source.join("jit_compiler_a64_static.S"))
                    .compile_intermediates(),
            );
        }
        _ => (), // The upstream interpreter remains available without a JIT backend.
    }
    cpp.cpp(true)
        .std("c++11")
        .files(CPP_SOURCES.iter().map(|file| source.join(file)))
        .objects(objects)
        .compile("randomx");
    if os == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
    } else {
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=m");
    }
}
