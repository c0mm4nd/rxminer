//! Differential tests against the exact native archive built with this checkout.
use rxminer::randomx_core::{
    blake2b::{self, Blake2b},
    superscalar::reciprocal,
};
use std::ffi::{c_int, c_void};

unsafe extern "C" {
    fn randomx_blake2b(
        out: *mut c_void,
        outlen: usize,
        input: *const c_void,
        inlen: usize,
        key: *const c_void,
        keylen: usize,
    ) -> c_int;
    fn randomx_blake2b_long(
        out: *mut c_void,
        outlen: usize,
        input: *const c_void,
        inlen: usize,
    ) -> c_int;
    fn randomx_reciprocal(divisor: u32) -> u64;
}

fn native_hash(input: &[u8], output: &mut [u8], long: bool) {
    // SAFETY: disjoint live slices carry their exact lengths; ordinary digests
    // are 1..=64 bytes, H' outputs are nonempty, and no key is supplied.
    let result = unsafe {
        if long {
            randomx_blake2b_long(
                output.as_mut_ptr().cast(),
                output.len(),
                input.as_ptr().cast(),
                input.len(),
            )
        } else {
            randomx_blake2b(
                output.as_mut_ptr().cast(),
                output.len(),
                input.as_ptr().cast(),
                input.len(),
                std::ptr::null(),
                0,
            )
        }
    };
    assert_eq!(result, 0);
}

fn random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn blake2b_matches_native_for_lengths_and_unaligned_buffers() {
    let mut data = [0; 4104];
    let mut state = 0x7479bf72859b0931;
    let lengths: Vec<_> = (0..=260)
        .chain([511, 512, 513, 1023, 1024, 1025, 4096])
        .collect();
    let mut cases = 0;
    for offset in 0..8 {
        for byte in &mut data {
            *byte = random(&mut state) as u8;
        }
        for &length in &lengths {
            let input = &data[offset..offset + length];
            for output_len in 1..=64 {
                let mut actual = [0xa5; 80];
                let mut expected = [0; 64];
                native_hash(input, &mut expected[..output_len], false);
                blake2b::hash(input, &mut actual[offset..offset + output_len]).unwrap();
                assert_eq!(
                    &actual[offset..offset + output_len],
                    &expected[..output_len],
                    "input={length} output={output_len} offset={offset}"
                );
                assert!(actual[..offset]
                    .iter()
                    .chain(&actual[offset + output_len..])
                    .all(|&byte| byte == 0xa5));
                cases += 1;
            }
        }
    }
    println!("PASSED Rust/native BLAKE2b: {cases} cases");
}

#[test]
fn streaming_and_argon2_expansion_match_native() {
    let mut data = [0; 4097];
    let mut state = 0x88b6dc162737d001;
    for byte in &mut data {
        *byte = random(&mut state) as u8;
    }
    let mut cases = 0;
    for length in [
        0, 1, 4, 60, 64, 76, 124, 125, 127, 128, 129, 256, 1024, 4097,
    ] {
        let input = &data[..length];
        for output_len in [1, 32, 64] {
            let mut expected = [0; 64];
            native_hash(input, &mut expected[..output_len], false);
            for chunk_size in [1, 7, 64, 127, 128, 129, 1024] {
                let mut hasher = Blake2b::new(output_len).unwrap();
                for chunk in input.chunks(chunk_size) {
                    hasher.update(chunk);
                    hasher.update(&[]);
                }
                let mut actual = [0; 64];
                hasher.finalize(&mut actual[..output_len]).unwrap();
                assert_eq!(
                    actual, expected,
                    "input={length} output={output_len} chunk={chunk_size}"
                );
                cases += 1;
            }
        }
        for output_len in [
            1, 2, 31, 32, 33, 63, 64, 65, 66, 95, 96, 97, 127, 128, 129, 255, 256, 257, 1024, 2048,
        ] {
            let mut actual = vec![0; output_len];
            let mut expected = vec![0; output_len];
            native_hash(input, &mut expected, true);
            blake2b::long_hash(input, &mut actual).unwrap();
            assert_eq!(actual, expected, "H' input={length} output={output_len}");
            cases += 1;
        }
    }
    println!("PASSED Rust/native streaming and Argon2 H': {cases} cases");
}

#[test]
fn reciprocals_match_native_at_boundaries_and_random_divisors() {
    let mut divisors = vec![0, 1, 2, 3, u32::MAX];
    for bit in 0..32 {
        let n = 1u32 << bit;
        divisors.extend([n - 1, n, n + 1]);
    }
    let mut state = 0x5782e503bc71d901;
    divisors.extend((0..65536).map(|_| random(&mut state) as u32));
    for divisor in divisors {
        if divisor == 0 || divisor.is_power_of_two() {
            assert_eq!(reciprocal(divisor), None);
        } else {
            // SAFETY: the native function accepts every nonzero divisor here;
            // no pointers or shared state are involved.
            let expected = unsafe { randomx_reciprocal(divisor) };
            assert_eq!(reciprocal(divisor), Some(expected), "divisor={divisor}");
        }
    }
}
