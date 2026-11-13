//! Identical public-API contract compiled separately with each backend feature.
use backend::{Dataset, MemoryMode, Tuning, Vm};
#[cfg(all(feature = "randomx-cpp", not(feature = "randomx-rust")))]
use randomx_cpp as backend;
#[cfg(feature = "randomx-rust")]
use randomx_rust as backend;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
const INPUT: &[u8] = b"This is a test";
const EXPECTED: [&str; 2] = [
    "639183aae1bf4c9a35884cb46b09cad9175f04efd7684e7262a0ac1c2f0b4e3f",
    "22ec6b861b3eb23686b2efbad69513c967ecfce80983df66c9c5b4fbfb4cdb6f",
];
fn data(mode: MemoryMode, key: &[u8]) -> Arc<Dataset> {
    Arc::new(
        Dataset::build_with_tuning(
            key,
            8,
            &AtomicBool::new(false),
            false,
            Tuning {
                mode,
                ..Tuning::default()
            },
            |_| Ok(()),
        )
        .unwrap(),
    )
}
fn check(vm: &mut Vm) {
    for v2 in [false, true] {
        assert_eq!(
            hex::encode(vm.hash(INPUT, v2).unwrap()),
            EXPECTED[usize::from(v2)]
        );
    }
    vm.first(INPUT);
    assert_eq!(
        hex::encode(vm.next_version(INPUT, false).unwrap()),
        EXPECTED[0]
    );
    assert_eq!(
        hex::encode(vm.next_version(INPUT, true).unwrap()),
        EXPECTED[1]
    );
    assert_eq!(hex::encode(vm.last_version(false).unwrap()), EXPECTED[0]);
}
#[test]
fn rejects_cancelled_empty_and_failed_setup() {
    let calls = AtomicUsize::new(0);
    for (threads, stop) in [(0, false), (1, true)] {
        assert!(Dataset::build_with_tuning(
            b"key",
            threads,
            &AtomicBool::new(stop),
            false,
            Tuning::default(),
            |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        )
        .is_err());
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    let error = Dataset::build_with_tuning(
        b"key",
        1,
        &AtomicBool::new(false),
        false,
        Tuning::default(),
        |_| anyhow::bail!("injected setup error"),
    );
    assert!(error
        .err()
        .unwrap()
        .to_string()
        .contains("injected setup error"));
}
#[test]
fn selected_backend_light_vectors_pipeline_and_replacement() {
    backend::set_thp(false);
    backend::set_memory_pool(0);
    let dataset = data(MemoryMode::Light, b"test key 000");
    assert!(dataset.is_light());
    assert!(!dataset.uses_huge_pages() && !dataset.uses_giga_pages());
    for jit in [false, true] {
        if jit && !backend::JIT_SUPPORTED {
            continue;
        }
        let mut vm = Vm::with_tuning(
            dataset.clone(),
            Tuning {
                jit,
                hw_aes: Some(false),
                ..Tuning::default()
            },
        )
        .unwrap();
        check(&mut vm);
        vm.first(INPUT);
        assert!(vm.set_dataset(dataset.clone()).is_err());
        assert_eq!(hex::encode(vm.last().unwrap()), EXPECTED[0]);
        vm.set_dataset(dataset.clone()).unwrap();
        check(&mut vm);
    }
    let mut vm = Vm::new(dataset.clone()).unwrap();
    let replacement = data(MemoryMode::Light, b"replacement seed");
    let expected = Vm::new(replacement.clone())
        .unwrap()
        .hash(INPUT, true)
        .unwrap();
    vm.set_dataset(replacement).unwrap();
    assert_eq!(vm.hash(INPUT, true).unwrap(), expected);
    vm.set_dataset(dataset).unwrap();
    check(&mut vm);
}
#[test]
#[ignore = "2080 MiB dataset; complete full/light, JIT/interpreter, AES, secure and prefetch matrix"]
fn selected_backend_full_mode_matrix() {
    backend::set_thp(false);
    backend::set_memory_pool(0);
    let mut count = 0;
    for mode in [MemoryMode::Fast, MemoryMode::Light] {
        let dataset = data(mode, b"test key 000");
        assert_eq!(dataset.is_light(), mode == MemoryMode::Light);
        for jit in [false, true] {
            if jit && !backend::JIT_SUPPORTED {
                continue;
            }
            for hw in [false, true] {
                for secure in [false, true] {
                    if secure && !jit {
                        continue;
                    }
                    for prefetch in 0..4 {
                        if (!jit || !cfg!(target_arch = "x86_64")) && prefetch != 1 {
                            continue;
                        }
                        let tuning = Tuning {
                            jit,
                            secure,
                            hw_aes: Some(hw),
                            ..Tuning::default()
                        };
                        if tuning.validate().is_err() {
                            continue;
                        }
                        backend::configure_jit(false, prefetch);
                        let mut vm = Vm::with_tuning(dataset.clone(), tuning).unwrap();
                        check(&mut vm);
                        vm.set_dataset(dataset.clone()).unwrap();
                        check(&mut vm);
                        count += 2;
                    }
                }
            }
        }
    }
    eprintln!(
        "{}: {count} version/configuration combinations",
        backend::BACKEND
    );
    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("aes") {
        assert_eq!(count, 72);
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
fn pipeline_restores_exact_mxcsr() {
    struct Restore(u32);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: saved valid control word; SSE2 is baseline on x86-64.
            unsafe {
                std::arch::asm!("ldmxcsr [{}]", in(reg) &self.0, options(nostack, preserves_flags));
            }
        }
    }
    let dataset = data(MemoryMode::Light, b"test key 000");
    let mut old = 0u32;
    // SAFETY: complete writable control word; preserves all caller state.
    unsafe {
        std::arch::asm!("stmxcsr [{}]", in(reg) &mut old, options(nostack, preserves_flags));
    }
    let _restore = Restore(old);
    for jit in [false, true] {
        for v2 in [false, true] {
            let mut vm = Vm::with_tuning(
                dataset.clone(),
                Tuning {
                    jit,
                    ..Tuning::default()
                },
            )
            .unwrap();
            let initial = 0xffe1u32;
            // SAFETY: all exceptions masked, valid rounding/DAZ/FTZ/status bits.
            unsafe {
                std::arch::asm!("ldmxcsr [{}]", in(reg) &initial, options(nostack, preserves_flags));
            }
            vm.first(INPUT);
            let next = vm.next_version(INPUT, v2).unwrap();
            let mut after = 0u32;
            // SAFETY: complete output word, read current control state.
            unsafe {
                std::arch::asm!("stmxcsr [{}]", in(reg) &mut after, options(nostack, preserves_flags));
            }
            assert_eq!(after, initial);
            assert_eq!(hex::encode(next), EXPECTED[usize::from(v2)]);
            let last = vm.last_version(v2).unwrap();
            // SAFETY: complete output word, read current control state.
            unsafe {
                std::arch::asm!("stmxcsr [{}]", in(reg) &mut after, options(nostack, preserves_flags));
            }
            assert_eq!(after, initial);
            assert_eq!(hex::encode(last), EXPECTED[usize::from(v2)]);
        }
    }
}

#[test]
fn auto_fallback_reuses_seed_and_cancellation_is_not_fallback() {
    for mode in [MemoryMode::Auto, MemoryMode::Fast] {
        let calls = AtomicUsize::new(0);
        let result = Dataset::build_with_tuning(
            b"test key 000",
            2,
            &AtomicBool::new(false),
            false,
            Tuning {
                mode,
                ..Tuning::default()
            },
            |_| {
                if calls.fetch_add(1, Ordering::Relaxed) > 0 {
                    anyhow::bail!("injected worker failure");
                }
                Ok(())
            },
        );
        if mode == MemoryMode::Auto {
            let dataset = Arc::new(result.unwrap());
            assert!(dataset.is_light());
            check(&mut Vm::new(dataset).unwrap());
        } else {
            assert!(result
                .err()
                .unwrap()
                .to_string()
                .contains("injected worker failure"));
        }
    }
    let calls = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let result = Dataset::build_with_tuning(
        b"key",
        1,
        &stop,
        false,
        Tuning {
            mode: MemoryMode::Auto,
            ..Tuning::default()
        },
        |_| {
            if calls.fetch_add(1, Ordering::Relaxed) > 0 {
                stop.store(true, Ordering::Relaxed);
            }
            Ok(())
        },
    );
    assert!(result.is_err());
}

#[test]
fn commitment_vectors() {
    let expected = [
        "d53ccf348b75291b7be76f0a7ac8208bbced734b912f6fca60539ab6f86be919",
        "133be717399046b03ae82ce8ddd9d1ee4d3ea7fca03a50dec09b6848cbb98e18",
    ];
    for (hash, wanted) in EXPECTED.iter().zip(expected) {
        let hash: [u8; 32] = hex::decode(hash).unwrap().try_into().unwrap();
        assert_eq!(hex::encode(backend::commitment(INPUT, &hash)), wanted);
    }
}
