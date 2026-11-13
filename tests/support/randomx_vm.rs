use anyhow::{ensure, Result};
use rxminer::randomx_core::{
    aes::{self, Aes},
    blake2b::Blake2b,
    cache::Cache,
    float::{self, Rounding},
    vm::Vm,
};
use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
};

fn bytes<const N: usize>(input: &mut Cursor<Vec<u8>>) -> Result<[u8; N]> {
    let mut result = [0; N];
    input.read_exact(&mut result)?;
    Ok(result)
}
fn u32(input: &mut Cursor<Vec<u8>>) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes(input)?))
}
fn u64(input: &mut Cursor<Vec<u8>>) -> Result<u64> {
    Ok(u64::from_le_bytes(bytes(input)?))
}
fn vector(input: &mut Cursor<Vec<u8>>, length: usize) -> Result<Vec<u8>> {
    let mut result = vec![0; length];
    input.read_exact(&mut result)?;
    Ok(result)
}

pub fn compare(path: &Path) -> Result<()> {
    let mut input = Cursor::new(fs::read(path)?);
    ensure!(&bytes::<8>(&mut input)? == b"RXVM0001", "invalid VM oracle");
    let count = u32(&mut input)?;
    ensure!(count == 16384, "incomplete float corpus");
    for case in 0..count {
        let mode = Rounding::from_bits(u64::from(u32(&mut input)?));
        let a = u64(&mut input)?;
        let b = u64(&mut input)?;
        let actual = [
            float::add(a, b, mode),
            float::sub(a, b, mode),
            float::mul(a, b, mode),
            float::div(a, b, mode),
            float::sqrt(a & 0x7fffffffffffffff, mode),
        ];
        for (op, actual) in actual.into_iter().enumerate() {
            let expected = u64(&mut input)?;
            ensure!(actual==expected,"float case={case} op={op} mode={mode:?} a={a:x} b={b:x} actual={actual:x} expected={expected:x}");
        }
    }
    let count = u32(&mut input)?;
    ensure!(count == 4096, "incomplete AES corpus");
    for case in 0..count {
        let state = bytes(&mut input)?;
        let key = bytes(&mut input)?;
        for decrypt in [false, true] {
            ensure!(
                aes::round(state, key, decrypt) == bytes::<16>(&mut input)?,
                "AES round case={case} decrypt={decrypt}"
            );
        }
    }
    let count = u32(&mut input)?;
    ensure!(count == 8, "incomplete AES buffers");
    let aes = Aes::default();
    for _ in 0..count {
        let size = u32(&mut input)? as usize;
        ensure!(
            size <= 2097152 && size.is_multiple_of(64),
            "invalid AES size"
        );
        let seed = bytes(&mut input)?;
        let expected_state = bytes::<64>(&mut input)?;
        let expected = vector(&mut input, size)?;
        let mut actual = vec![0; size];
        let mut state = seed;
        aes.fill(&mut state, &mut actual);
        ensure!(
            state == expected_state && actual == expected,
            "AES fill size={size}"
        );
        ensure!(
            aes.hash(&actual) == bytes::<64>(&mut input)?,
            "AES hash size={size}"
        );
        aes.program(&seed, &mut actual);
        ensure!(
            actual == vector(&mut input, size)?,
            "AES program size={size}"
        );
    }
    let count = u32(&mut input)?;
    ensure!(count == 4, "incomplete cache corpus");
    let mut cache: Option<std::sync::Arc<Cache>> = None;
    let mut vm = Vm::new()?;
    for key_index in 0..count {
        let length = u32(&mut input)? as usize;
        ensure!(length <= 80, "invalid key size");
        let key = vector(&mut input, length)?;
        match &mut cache {
            Some(cache) => std::sync::Arc::get_mut(cache).unwrap().reinitialize(&key)?,
            None => cache = Some(std::sync::Arc::new(Cache::new(&key)?)),
        }
        let cache = cache.as_ref().unwrap();
        let mut hasher = Blake2b::new(64)?;
        let mut buffer = [0; 1024];
        for block in cache.words().chunks(128) {
            for (out, word) in buffer.as_chunks_mut::<8>().0.iter_mut().zip(block) {
                *out = word.to_le_bytes();
            }
            hasher.update(&buffer);
        }
        let mut digest = [0; 64];
        hasher.finalize(&mut digest)?;
        ensure!(
            digest == bytes::<64>(&mut input)?,
            "cache digest key={key_index}"
        );
        let items = u32(&mut input)?;
        ensure!(items == 1024, "incomplete dataset corpus");
        for _ in 0..items {
            let index = u64(&mut input)?;
            let actual = cache.dataset_item(index);
            for word in actual {
                ensure!(
                    word == u64(&mut input)?,
                    "dataset key={key_index} item={index}"
                );
            }
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let accelerated = std::sync::Arc::new(rxminer::randomx_jit::new_cache(&key, None)?);
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        ensure!(
            accelerated.words() == cache.words(),
            "AVX2 cache key={key_index}"
        );
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let mut jits = [false, true].map(|hardware| {
            rxminer::randomx_jit::Vm::new(
                rxminer::randomx_jit::Source::Light(accelerated.clone()),
                hardware,
            )
            .unwrap()
        });
        let hashes = u32(&mut input)?;
        ensure!(hashes == 6, "incomplete hash corpus");
        for case in 0..hashes {
            let v2 = u32(&mut input)? != 0;
            let length = u32(&mut input)? as usize;
            ensure!(length <= 257, "invalid input size");
            let data = vector(&mut input, length)?;
            let expected = bytes::<32>(&mut input)?;
            ensure!(
                vm.hash(&**cache, &data, v2) == expected,
                "full hash key={key_index} case={case} v2={v2}"
            );
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            for jit in &mut jits {
                ensure!(
                    jit.hash(&data, v2)? == expected,
                    "Rust JIT key={key_index} case={case} v2={v2}"
                );
            }
        }
    }
    ensure!(
        input.position() == input.get_ref().len() as u64,
        "unread VM oracle data"
    );
    println!("PASSED Rust VM: 81920 floating-point results, 8192 AES rounds, full AES buffers, four 256 MiB cache digests, 4096 dataset items and 24 complete v1/v2 hashes (also checked with Rust JIT/software and hardware AES on Linux x86-64)");
    Ok(())
}
