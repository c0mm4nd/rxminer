// SPDX-License-Identifier: BSD-3-Clause
//! ARM64 and RV64 instruction, chain and public-mode differentials.
use super::*;
#[test]
#[ignore = "256 MiB cache and all eight programs compared"]
fn chain_differential() {
    let cache = Arc::new(new_cache(b"test key 000", None).unwrap());
    for v2 in [false, true] {
        let mut reference = crate::randomx_core::vm::Vm::new().unwrap();
        reference.first(b"This is a test");
        let mut jit = Vm::new(Source::Light(cache.clone()), false).unwrap();
        jit.first(b"This is a test");
        let mut seed = jit.pending.unwrap();
        let mut rounding = crate::randomx_core::float::Rounding::Nearest;
        let _guard = RoundingGuard::new();
        for stage in 0..8 {
            let mut program = vec![0; 128 + if v2 { 384 * 8 } else { 256 * 8 }];
            Aes::default().program(&seed, &mut program);
            let expected = reference.trace_chain_program(&*cache, &program, v2, &mut rounding);
            let actual = jit.run(&seed, v2).unwrap();
            assert_eq!(actual.bytes(), expected.0, "stage={stage} v2={v2}");
            assert_eq!(
                jit.scratch.bytes(),
                expected.1,
                "scratch stage={stage} v2={v2}"
            );
            #[cfg(target_arch = "riscv64")]
            let fcsr: u64;
            // SAFETY: read the current userspace FP control register for differential diagnostics.
            #[cfg(target_arch = "riscv64")]
            unsafe {
                core::arch::asm!("frcsr {}",out(reg) fcsr,options(nostack,preserves_flags));
            }
            #[cfg(target_arch = "riscv64")]
            eprintln!("rounding={rounding:?}, fcsr={fcsr:x}");
            seed = blake2b::hash_512(&actual.bytes());
            eprintln!("stage={stage} v2={v2} matches");
        }
    }
}
#[test]
#[ignore = "256 MiB cache and each emitted instruction executed against portable Rust"]
fn instruction_differential() {
    let cache = Arc::new(new_cache(b"test key 000", None).unwrap());
    let mut opcode = 0u16;
    for frequency in crate::randomx_core::vm::FREQUENCIES
        .into_iter()
        .chain(std::iter::repeat_n(1, 33))
    {
        for v2 in [false, true] {
            let size = 128 + if v2 { 384 * 8 } else { 256 * 8 };
            let mut program = vec![0; size];
            for (i, chunk) in program[128..].chunks_exact_mut(8).enumerate() {
                chunk.copy_from_slice(&[
                    opcode as u8,
                    (i % 8) as u8,
                    ((i * 3 + 1) % 8) as u8,
                    0x65,
                    0x71,
                    0x32,
                    0x91,
                    0xa3,
                ]);
            }
            if opcode >= 256 {
                Aes::default().program(&[opcode as u8; 64], &mut program);
            }
            let iterations = if opcode == 288 { 2048 } else { 3 };
            if opcode == 288 {
                let mut seed = blake2b::hash_512(b"This is a test");
                Aes::default().fill(&mut seed, &mut vec![0; 2 << 20]);
                Aes::default().program(&seed, &mut program);
            }
            let entropy =
                |i: usize| u64::from_le_bytes(program[8 * i..8 * i + 8].try_into().unwrap());
            let read = std::array::from_fn(|i| 2 * i + ((entropy(12) >> i) & 1) as usize);
            let offset = (entropy(13) % 524288) * 64;
            let masks = [entropy(14), entropy(15)]
                .map(|bits| (bits & ((1 << 22) - 1)) | ((0x300 | ((bits >> 60) << 4)) << 52));
            let mut reference = crate::randomx_core::vm::Vm::new().unwrap();
            reference.first(b"This is a test");
            let expected = reference.trace_program(&*cache, &program, v2, iterations);
            let mut jit = Vm::new(Source::Light(cache.clone()), false).unwrap();
            jit.first(b"This is a test");
            jit.emitter.compile(
                &program[128..],
                emitter::ProgramConfig {
                    read,
                    #[cfg(target_arch = "riscv64")]
                    masks,
                    #[cfg(target_arch = "aarch64")]
                    hardware: false,
                    offset: offset as u32,
                    v2,
                    light: true,
                },
            );
            jit.code.publish(&jit.emitter.code[..PROGRAM_SIZE]).unwrap();
            let mut registers = Registers::default();
            for i in 0..8 {
                let bits = entropy(i);
                registers.words[24 + i] =
                    (((bits >> 59) + 1023) << 52) | (bits & ((1u64 << 52) - 1));
            }
            #[cfg(target_arch = "aarch64")]
            registers.words[8..10].copy_from_slice(&masks);
            let memory = Memory {
                mx: entropy(10) as u32,
                ma: entropy(8) as u32 & 2147483584,
                pointer: cache.words().as_ptr().cast(),
            };
            let _rounding = RoundingGuard::new();
            // SAFETY: generated template and bounded cache/scratch/ABI buffers, the selected number of iterations.
            unsafe {
                let function: unsafe extern "C" fn(*mut Registers, *const Memory, *mut u8, u64) =
                    std::mem::transmute(jit.code.entry().add(Emitter::entry()));
                function(
                    &mut registers,
                    &memory,
                    jit.scratch.bytes_mut().as_mut_ptr(),
                    iterations as u64,
                );
            }
            assert_eq!(registers.bytes(), expected.0, "opcode={opcode} v2={v2}");
            assert_eq!(
                jit.scratch.bytes(),
                expected.1,
                "scratch opcode={opcode} v2={v2}"
            );
            eprintln!("opcode={opcode} v2={v2} matches");
        }
        opcode += frequency;
    }
}

#[test]
#[ignore = "full dataset and all full/light, soft/hard AES and secure pipelines"]
fn platform_modes_and_replacement() {
    let cache = Arc::new(new_cache(b"test key 000", None).unwrap());
    let init = Initializer::new(&cache).unwrap();
    for start in [0, 12345, crate::randomx_core::cache::DATASET_ITEMS - 17] {
        let mut out = [[0; 8]; 17];
        init.fill(start, &mut out).unwrap();
        for (i, item) in out.iter().enumerate() {
            assert_eq!(*item, cache.dataset_item((start + i) as u64));
        }
    }
    let data = Arc::new(
        init.build_dataset(4, &std::sync::atomic::AtomicBool::new(false), |_| Ok(()))
            .unwrap(),
    );
    let inputs = [
        b"This is a test".as_slice(),
        b"Lorem ipsum dolor sit amet",
        b"another input",
    ];
    let versions = [false, true, false];
    let mut reference = crate::randomx_core::vm::Vm::new().unwrap();
    let expected: Vec<_> = inputs
        .iter()
        .zip(versions)
        .map(|(input, v2)| reference.hash(&*cache, input, v2))
        .collect();
    for full in [false, true] {
        for hardware in [false, true] {
            if hardware && HardwareAes::new().is_none() {
                continue;
            }
            for secure in [false, true] {
                let source = || {
                    if full {
                        Source::Full(data.clone())
                    } else {
                        Source::Light(cache.clone())
                    }
                };
                let mut vm = Vm::with_options(
                    source(),
                    VmOptions {
                        hardware_aes: hardware,
                        secure,
                        huge_pages: true,
                        jit_huge_pages: true,
                        ..Default::default()
                    },
                )
                .unwrap();
                vm.first(inputs[0]);
                assert!(vm.set_source(source()).is_err());
                for i in 0..inputs.len() {
                    let actual = if i + 1 < inputs.len() {
                        vm.next(inputs[i + 1], versions[i])
                    } else {
                        vm.last(versions[i])
                    }
                    .unwrap();
                    assert_eq!(
                        actual, expected[i],
                        "full={full} hardware={hardware} secure={secure} input={i}"
                    );
                }
                vm.set_source(source()).unwrap();
                assert_eq!(vm.hash(inputs[0], false).unwrap(), expected[0]);
            }
        }
    }
    let changed = Arc::new(new_cache(b"replacement seed", None).unwrap());
    let mut vm = Vm::new(Source::Light(cache.clone()), false).unwrap();
    for source in [&changed, &cache] {
        vm.set_source(Source::Light(source.clone())).unwrap();
        for v2 in [false, true] {
            assert_eq!(
                vm.hash(inputs[0], v2).unwrap(),
                reference.hash(&**source, inputs[0], v2)
            );
        }
    }
}
