#![cfg(feature = "randomx-rust")]
//! Explicit differential matrix against a separately built C++ reference.
#![cfg(all(target_arch = "x86_64", any(unix, windows)))]
use rxminer::{
    randomx::{configure_jit, Dataset, Tuning, Vm},
    randomx_memory::Policy,
};
use std::ffi::c_void;
use std::sync::{atomic::AtomicBool, Arc};
#[test]
#[ignore = "complete dataset and C++ oracle; RXMINER_NATIVE_LIBRARY required"]
fn all_modes_match_cpp() {
    let path = std::env::var_os("RXMINER_NATIVE_LIBRARY").expect("native oracle path");
    // SAFETY: explicit developer oracle. Function pointers remain valid until
    // all native handles have been destroyed and the library is dropped.
    let library = unsafe { libloading::Library::new(path).unwrap() };
    type Alloc = unsafe extern "C" fn(u32) -> *mut c_void;
    type Init = unsafe extern "C" fn(*mut c_void, *const u8, usize);
    type Create = unsafe extern "C" fn(u32, *mut c_void, *mut c_void) -> *mut c_void;
    type Free = unsafe extern "C" fn(*mut c_void);
    type Hash = unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut u8);
    type Commitment = unsafe extern "C" fn(*const u8, usize, *const u8, *mut u8);
    // SAFETY: signatures exactly match the retained randomx.h API.
    let (alloc, init, create, free_cache, free_vm, hash, commitment) = unsafe {
        (
            *library.get::<Alloc>(b"randomx_alloc_cache\0").unwrap(),
            *library.get::<Init>(b"randomx_init_cache\0").unwrap(),
            *library.get::<Create>(b"randomx_create_vm\0").unwrap(),
            *library.get::<Free>(b"randomx_release_cache\0").unwrap(),
            *library.get::<Free>(b"randomx_destroy_vm\0").unwrap(),
            *library.get::<Hash>(b"randomx_calculate_hash\0").unwrap(),
            *library
                .get::<Commitment>(b"randomx_calculate_commitment\0")
                .unwrap(),
        )
    };
    struct Owned(*mut c_void, Free);
    impl Drop for Owned {
        fn drop(&mut self) {
            if self.0.is_null() {
                return;
            }
            // SAFETY: this owns one live handle with its matching destructor.
            unsafe {
                (self.1)(self.0);
            }
        }
    }
    let key = b"test key 000";
    let inputs = [b"This is a test".as_slice(), b"Lorem ipsum dolor sit amet"];
    // SAFETY: supported cache flags, no borrowed parameters.
    let native = Owned(unsafe { alloc(8 | 96) }, free_cache);
    assert!(!native.0.is_null());
    // SAFETY: complete input slice and freshly allocated cache.
    unsafe {
        init(native.0, key.as_ptr(), key.len());
    }
    let mut expected = [[[0; 32]; 2]; 2];
    for v2 in [false, true] {
        // Check both C++ interpreter and JIT, each with soft and hard AES.
        for flags in [0, 2, 8, 10, 24, 26] {
            // SAFETY: cache remains live for this light VM; no dataset required.
            let vm = Owned(
                unsafe {
                    create(
                        flags | if v2 { 128 } else { 0 },
                        native.0,
                        std::ptr::null_mut(),
                    )
                },
                free_vm,
            );
            assert!(!vm.0.is_null());
            for (i, input) in inputs.iter().enumerate() {
                let mut result = [0; 32];
                // SAFETY: live VM, disjoint input and complete output buffer.
                unsafe {
                    hash(vm.0, input.as_ptr(), input.len(), result.as_mut_ptr());
                }
                if flags == 0 {
                    expected[v2 as usize][i] = result;
                } else {
                    assert_eq!(result, expected[v2 as usize][i]);
                }
                let mut com = [0; 32];
                // SAFETY: complete disjoint input, hash and output buffers.
                unsafe {
                    commitment(
                        input.as_ptr(),
                        input.len(),
                        result.as_ptr(),
                        com.as_mut_ptr(),
                    );
                }
                assert_eq!(rxminer::randomx_core::commitment(input, &result), com);
            }
        }
    }
    drop(native);
    let cache = Arc::new(rxminer::randomx_jit::new_cache(key, Some(32)).unwrap());
    let data = Arc::new(
        rxminer::randomx_jit::Initializer::new(&cache)
            .unwrap()
            .build_dataset_with_policy(
                8,
                &AtomicBool::new(false),
                |_| Ok(()),
                Policy {
                    huge: true,
                    giga: true,
                },
            )
            .unwrap(),
    );
    let mut count = 0;
    for full in [false, true] {
        for jit in [false, true] {
            for hardware in [false, true] {
                for secure in [false, true] {
                    if !jit && secure {
                        continue;
                    }
                    for prefetch in 0..4 {
                        if !jit && prefetch != 1 {
                            continue;
                        }
                        configure_jit(true, prefetch);
                        let tuning = Tuning {
                            jit,
                            secure,
                            hw_aes: Some(hardware),
                            ..Tuning::default()
                        };
                        let dataset = Arc::new(if full {
                            Dataset::from_dataset(data.clone(), true, tuning).unwrap()
                        } else {
                            Dataset::from_cache(cache.clone(), true, tuning).unwrap()
                        });
                        let mut vm = Vm::new(dataset.clone()).unwrap();
                        for v2 in [false, true] {
                            vm.first(inputs[0]);
                            assert!(vm.set_dataset(dataset.clone()).is_err());
                            assert_eq!(vm.next_version(inputs[1], v2).unwrap(), expected[v2 as usize][0],
                                "full={full} jit={jit} aes={hardware} secure={secure} prefetch={prefetch} v2={v2}");
                            assert_eq!(vm.last_version(v2).unwrap(), expected[v2 as usize][1]);
                            vm.set_dataset(dataset.clone()).unwrap();
                            assert_eq!(vm.hash(inputs[0], v2).unwrap(), expected[v2 as usize][0]);
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    configure_jit(false, 1);
    println!("PASSED {count} Rust mode combinations, pipelines and source replacement against C++");
}
