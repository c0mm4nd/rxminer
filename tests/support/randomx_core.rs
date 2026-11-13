use anyhow::{ensure, Result};
use rxminer::randomx_core::{
    generator::Blake2Generator,
    scheduler::{self, GeneratedProgram},
    superscalar::Program,
};
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
    ensure!(&magic == b"RXRUST02", "invalid native oracle format");
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
        compare_program(&mut input, case, None)?;
    }
    let chains = u32(&mut input)?;
    ensure!(chains == 256, "incomplete scheduler corpus");
    for chain in 0..chains {
        let len = u32(&mut input)? as usize;
        ensure!(len <= 255, "invalid scheduler seed size");
        let mut seed = vec![0; len];
        input.read_exact(&mut seed)?;
        let nonce = u32(&mut input)?;
        let count = u32(&mut input)?;
        ensure!(count == 8, "incomplete cache program chain");
        let mut generator = Blake2Generator::new(&seed, nonce);
        for index in 0..count {
            let case = chain * count + index;
            let generated = scheduler::generate(&mut generator);
            compare_program(&mut input, case, Some(&generated))?;
            let stats = generated.schedule();
            for value in [
                stats.code_size,
                stats.macro_ops,
                stats.decode_cycles,
                stats.cpu_latency,
                stats.asic_latency,
                stats.mul_count,
            ]
            .into_iter()
            .chain(stats.cpu_latencies)
            .chain(stats.asic_latencies)
            {
                ensure!(
                    value == u32(&mut input)? as usize,
                    "schedule mismatch case={case}"
                );
            }
            let mut continuation = generator.clone();
            for _ in 0..32 {
                ensure!(
                    continuation.next_u32() == u32(&mut input)?,
                    "generator consumption mismatch case={case}"
                );
            }
        }
    }
    println!("PASSED Rust/C++ scheduler: {} programs, exact encoding, address register, scheduling, execution and remaining byte stream", chains * 8);
    ensure!(
        input.position() == input.get_ref().len() as u64,
        "unread oracle data"
    );
    println!("PASSED Rust/C++ migration differential: {streams} generator streams, {programs} superscalar programs in both reciprocal modes");
    Ok(())
}

fn compare_program(
    input: &mut Cursor<Vec<u8>>,
    case: u32,
    generated: Option<&GeneratedProgram>,
) -> Result<()> {
    let size = u32(input)? as usize;
    ensure!((1..=512).contains(&size), "invalid program size");
    let address = u32(input)?;
    ensure!(address < 8, "invalid address register");
    let mut encoded = vec![0; size * 8];
    input.read_exact(&mut encoded)?;
    if let Some(generated) = generated {
        ensure!(
            generated.encoded() == encoded,
            "generated instruction mismatch case={case}"
        );
        ensure!(
            u32::from(generated.address_register()) == address,
            "generated address mismatch case={case}"
        );
    }
    let raw = Program::decode(&encoded, address as u8, None)?;
    ensure!(
        raw.address_register() == address as usize,
        "address register mismatch"
    );
    let initial = registers(input)?;
    let expected = registers(input)?;
    if let Some(generated) = generated {
        let mut actual = initial;
        generated.prepare().execute(&mut actual);
        ensure!(
            actual == expected,
            "generated execution mismatch case={case}"
        );
    }
    let mut actual = initial;
    raw.execute(&mut actual);
    ensure!(actual == expected, "raw superscalar mismatch case={case}");
    let count = u32(input)?;
    ensure!(count <= size as u32, "invalid reciprocal count");
    let mut reciprocals = Vec::new();
    for _ in 0..count {
        reciprocals.push(u64(input)?);
    }
    input.read_exact(&mut encoded)?;
    let cached = Program::decode(&encoded, address as u8, Some(&reciprocals))?;
    let expected_cached = registers(input)?;
    actual = initial;
    cached.execute(&mut actual);
    ensure!(
        actual == expected && actual == expected_cached,
        "cached superscalar mismatch case={case}"
    );
    Ok(())
}
