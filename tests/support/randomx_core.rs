use anyhow::{ensure, Result};
use rxminer::randomx_core::{generator::Blake2Generator, superscalar::Program};
use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
};

fn u32(input: &mut Cursor<Vec<u8>>) -> Result<u32> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn u64(input: &mut Cursor<Vec<u8>>) -> Result<u64> {
    let mut bytes = [0; 8];
    input.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
fn registers(input: &mut Cursor<Vec<u8>>) -> Result<[u64; 8]> {
    let mut words = [0; 8];
    for word in &mut words {
        *word = u64(input)?;
    }
    Ok(words)
}

pub fn compare(path: &Path) -> Result<()> {
    let mut input = Cursor::new(fs::read(path)?);
    let mut magic = [0; 8];
    input.read_exact(&mut magic)?;
    ensure!(&magic == b"RXRUST01", "invalid native oracle format");
    let streams = u32(&mut input)?;
    ensure!(streams == 160, "incomplete generator corpus");
    for case in 0..streams {
        let len = u32(&mut input)? as usize;
        ensure!(len <= 255, "invalid seed size");
        let mut seed = vec![0; len];
        input.read_exact(&mut seed)?;
        let nonce = u32(&mut input)?;
        let operations = u32(&mut input)?;
        ensure!(operations == 512, "incomplete generator stream");
        let mut generator = Blake2Generator::new(&seed, nonce);
        for operation in 0..operations {
            let value = match u32(&mut input)? {
                1 => u32::from(generator.next_byte()),
                4 => generator.next_u32(),
                _ => anyhow::bail!("invalid read width"),
            };
            ensure!(
                value == u32(&mut input)?,
                "byte stream mismatch case={case} operation={operation}"
            );
        }
    }
    let programs = u32(&mut input)?;
    ensure!(programs == 3840, "incomplete superscalar corpus");
    for case in 0..programs {
        let size = u32(&mut input)? as usize;
        ensure!((1..=512).contains(&size), "invalid program size");
        let address = u32(&mut input)?;
        ensure!(address < 8, "invalid address register");
        let mut encoded = vec![0; size * 8];
        input.read_exact(&mut encoded)?;
        let raw = Program::decode(&encoded, address as u8, None)?;
        ensure!(
            raw.address_register() == address as usize,
            "address register mismatch"
        );
        let initial = registers(&mut input)?;
        let expected = registers(&mut input)?;
        let mut actual = initial;
        raw.execute(&mut actual);
        ensure!(actual == expected, "raw superscalar mismatch case={case}");
        let count = u32(&mut input)?;
        ensure!(count <= size as u32, "invalid reciprocal count");
        let mut reciprocals = Vec::new();
        for _ in 0..count {
            reciprocals.push(u64(&mut input)?);
        }
        input.read_exact(&mut encoded)?;
        let cached = Program::decode(&encoded, address as u8, Some(&reciprocals))?;
        let expected_cached = registers(&mut input)?;
        actual = initial;
        cached.execute(&mut actual);
        ensure!(
            actual == expected && actual == expected_cached,
            "cached superscalar mismatch case={case}"
        );
    }
    ensure!(
        input.position() == input.get_ref().len() as u64,
        "unread oracle data"
    );
    println!("PASSED Rust/C++ migration differential: {streams} generator streams, {programs} superscalar programs in both reciprocal modes");
    Ok(())
}
