use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;

pub const NONCE_OFFSET: usize = 39;
pub const MAX_BLOB: usize = 1024;

#[derive(Clone, Debug)]
pub struct Job {
    pub id: String,
    pub blob: Vec<u8>,
    pub seed: [u8; 32],
    pub target: u64,
    pub height: u64,
    pub dev: bool,
    pub session: u64,
    pub nicehash: bool,
}

#[derive(Deserialize)]
struct WireJob {
    job_id: String,
    blob: String,
    seed_hash: String,
    target: String,
    #[serde(default)]
    height: u64,
    #[serde(default = "algorithm")]
    algo: String,
}

fn algorithm() -> String {
    "rx/0".into()
}

impl Job {
    // Difficulty/height updates do not create new nonce work. Session, seed,
    // stream, job ID and blob do: never reuse progress across those boundaries.
    pub(crate) fn same_work(&self, other: &Self) -> bool {
        self.nicehash == other.nicehash
            && self.session == other.session
            && self.dev == other.dev
            && self.seed == other.seed
            && self.id == other.id
            && self.blob == other.blob
    }

    pub fn parse(value: serde_json::Value, dev: bool, session: u64) -> Result<Self> {
        let wire: WireJob = serde_json::from_value(value).context("invalid job fields")?;
        ensure!(wire.algo == "rx/0", "unsupported algorithm {}", wire.algo);
        ensure!(!wire.job_id.is_empty(), "empty job id");
        let blob = hex::decode(wire.blob).context("invalid job blob")?;
        ensure!(
            (NONCE_OFFSET + 4..=MAX_BLOB).contains(&blob.len()),
            "invalid blob length"
        );
        let seed = decode_seed(&wire.seed_hash)?;
        let target = parse_target(&wire.target)?;
        Ok(Self {
            id: wire.job_id,
            blob,
            seed,
            target,
            height: wire.height,
            dev,
            session,
            nicehash: false,
        })
    }

    pub fn nonce_base(&self) -> u32 {
        u32::from_le_bytes(
            self.blob[NONCE_OFFSET..NONCE_OFFSET + 4]
                .try_into()
                .unwrap(),
        )
    }

    pub fn nonce_limit(&self) -> u64 {
        if self.nicehash {
            1 << 24
        } else {
            1 << 32
        }
    }

    pub fn nonce_at(&self, cursor: u64) -> u32 {
        let base = self.nonce_base();
        if self.nicehash {
            (base & 0xff00_0000) | (base.wrapping_add(cursor as u32) & 0x00ff_ffff)
        } else {
            base.wrapping_add(cursor as u32)
        }
    }

    pub fn benchmark(seed: [u8; 32]) -> Self {
        let mut blob = vec![0x42; 76];
        blob[0] = 0x0c;
        blob[NONCE_OFFSET..NONCE_OFFSET + 4].fill(0);
        Self {
            id: "benchmark".into(),
            blob,
            seed,
            target: 0,
            height: 0,
            dev: false,
            session: 0,
            nicehash: false,
        }
    }
}

pub fn decode_seed(value: &str) -> Result<[u8; 32]> {
    hex::decode(value)
        .context("seed must be 64 hex characters")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("seed must be 64 hex characters"))
}

pub fn parse_target(value: &str) -> Result<u64> {
    let bytes = hex::decode(value).context("invalid target hex")?;
    let target = match bytes.len() {
        4 => {
            let value = u32::from_le_bytes(bytes.try_into().unwrap()) as u64;
            ensure!(value != 0, "zero target");
            u64::MAX / (u32::MAX as u64 / value)
        }
        8 => u64::from_le_bytes(bytes.try_into().unwrap()),
        _ => bail!("target must contain 4 or 8 bytes"),
    };
    ensure!(target != 0, "zero target");
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nicehash_nonce_wrap_preserves_prefix_and_work_identity() {
        let mut job = Job::benchmark([0; 32]);
        job.nicehash = true;
        job.blob[NONCE_OFFSET..NONCE_OFFSET + 4].copy_from_slice(&0xabfffffe_u32.to_le_bytes());
        assert_eq!(job.nonce_limit(), 1 << 24);
        assert_eq!(job.nonce_at(0), 0xabfffffe);
        assert_eq!(job.nonce_at(1), 0xabffffff);
        assert_eq!(job.nonce_at(2), 0xab000000);
        assert_eq!(job.nonce_at((1 << 24) - 1), 0xabfffffd);
        let mut ordinary = job.clone();
        ordinary.nicehash = false;
        assert!(!ordinary.same_work(&job));
        assert_eq!(ordinary.nonce_at(2), 0xac000000);
    }

    #[test]
    fn targets_are_little_endian_and_normalized() {
        assert_eq!(parse_target("ffffffff").unwrap(), u64::MAX);
        assert_eq!(parse_target("01000000").unwrap(), 0x100000001);
        assert_eq!(parse_target("0100000000000000").unwrap(), 1);
        for bad in ["", "00", "00000000", "0000000000000000", "gggggggg"] {
            assert!(parse_target(bad).is_err());
        }
    }
    #[test]
    fn invalid_jobs_cannot_reach_the_hash_loop() {
        let mut value = serde_json::json!({"job_id":"a", "blob":"42".repeat(76),
            "seed_hash":"00".repeat(32), "target":"ffffffff"});
        assert!(Job::parse(value.clone(), false, 1).is_ok());
        value["blob"] = "42".repeat(1025).into();
        assert!(Job::parse(value.clone(), false, 1).is_err());
        value["blob"] = "42".repeat(76).into();
        value["seed_hash"] = "00".into();
        assert!(Job::parse(value, false, 1).is_err());
    }
}
