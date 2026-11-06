//! Linux MSR changes with rollback on partial failure and restoration on exit.
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;
#[cfg(target_os = "linux")]
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{FileExt, OpenOptionsExt},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub register: u32,
    pub value: u64,
    pub mask: u64,
}
fn number(value: &str) -> Result<u64> {
    let value = value.trim();
    Ok(
        if let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            u64::from_str_radix(hex, 16)?
        } else {
            value.parse()?
        },
    )
}
fn entry(value: &str) -> Result<Entry> {
    let parts: Vec<_> = value.split(':').collect();
    ensure!(
        matches!(parts.len(), 2 | 3),
        "MSR entry must be register:value[:mask]"
    );
    let register = u32::try_from(number(parts[0])?).context("MSR register out of range")?;
    let mask = if parts.len() == 3 {
        number(parts[2])?
    } else {
        u64::MAX
    };
    ensure!(mask != 0, "MSR mask cannot be zero");
    Ok(Entry {
        register,
        value: number(parts[1])?,
        mask,
    })
}
#[cfg(target_arch = "x86_64")]
fn cpu() -> ([u8; 12], u32, u32) {
    // SAFETY: CPUID leaves 0 and 1 exist on all supported x86-64 CPUs.
    #[allow(unused_unsafe)] // CPUID was unsafe at our minimum Rust version.
    let (vendor, version) =
        // SAFETY: CPUID leaves 0 and 1 are supported on x86-64.
        unsafe { (std::arch::x86_64::__cpuid(0), std::arch::x86_64::__cpuid(1)) };
    let mut name = [0; 12];
    name[..4].copy_from_slice(&vendor.ebx.to_le_bytes());
    name[4..8].copy_from_slice(&vendor.edx.to_le_bytes());
    name[8..].copy_from_slice(&vendor.ecx.to_le_bytes());
    let base = (version.eax >> 8) & 15;
    (
        name,
        base + if base == 15 {
            (version.eax >> 20) & 255
        } else {
            0
        },
        ((version.eax >> 4) & 15) | ((version.eax >> 12) & 0xf0),
    )
}
#[cfg(not(target_arch = "x86_64"))]
fn cpu() -> ([u8; 12], u32, u32) {
    ([0; 12], 0, 0)
}
pub fn parse(value: &Value) -> Result<Vec<Entry>> {
    match value {
        Value::Bool(false) => Ok(vec![]),
        Value::Number(n) if n.as_i64() == Some(-1) => Ok(vec![]),
        Value::Bool(true) => {
            let (vendor, family, model) = cpu();
            let values: &[&str] = if &vendor == b"GenuineIntel" && family == 6 {
                &["0x1a4:0xf"]
            } else if &vendor == b"AuthenticAMD"
                && family == 0x17
                && matches!(model, 0x31 | 0x60 | 0x68 | 0x71)
            {
                &[
                    "0xc0011020:0x0",
                    "0xc0011021:0x40:0xffffffffffffffdf",
                    "0xc0011022:0x1510000",
                    "0xc001102b:0x2000cc16",
                ]
            } else if &vendor == b"AuthenticAMD"
                && family == 0x19
                && matches!(model, 0x00..=0x0f | 0x20..=0x2f | 0x50..=0x5f)
            {
                &[
                    "0xc0011020:0x4480000000000",
                    "0xc0011021:0x1c000200000040:0xffffffffffffffdf",
                    "0xc0011022:0xc000000401500000",
                    "0xc001102b:0x2000cc14",
                ]
            } else {
                bail!("no automatic MSR preset for this CPU; specify explicit registers");
            };
            values.iter().map(|v| entry(v)).collect()
        }
        Value::Number(n) => {
            ensure!(
                &cpu().0 == b"GenuineIntel",
                "numeric wrmsr is only supported on Intel"
            );
            let value = n
                .as_u64()
                .filter(|v| *v <= 15)
                .context("numeric wrmsr must be 0..15")?;
            Ok(vec![Entry {
                register: 0x1a4,
                value,
                mask: u64::MAX,
            }])
        }
        Value::Array(entries) => {
            ensure!(entries.len() <= 64, "too many MSR entries");
            let entries: Vec<Entry> = entries
                .iter()
                .map(|v| entry(v.as_str().context("MSR entries must be strings")?))
                .collect::<Result<_>>()?;
            let unique: std::collections::BTreeSet<_> =
                entries.iter().map(|e| e.register).collect();
            ensure!(unique.len() == entries.len(), "duplicate MSR register");
            Ok(entries)
        }
        _ => bail!("randomx.wrmsr must be boolean, an Intel value, or register strings"),
    }
}
trait Access {
    #[cfg(any(target_os = "linux", test))]
    fn read(&mut self, cpu: usize, register: u32) -> Result<u64>;
    fn write(&mut self, cpu: usize, register: u32, value: u64) -> Result<()>;
}
#[cfg(target_os = "linux")]
struct Device {
    files: BTreeMap<usize, File>,
    _lock: File,
}
#[cfg(target_os = "linux")]
impl Access for Device {
    fn read(&mut self, cpu: usize, register: u32) -> Result<u64> {
        let mut bytes = [0; 8];
        self.files[&cpu].read_exact_at(&mut bytes, register.into())?;
        Ok(u64::from_le_bytes(bytes))
    }
    fn write(&mut self, cpu: usize, register: u32, value: u64) -> Result<()> {
        let written = self.files[&cpu].write_at(&value.to_le_bytes(), register.into())?;
        ensure!(written == 8, "short MSR write");
        Ok(())
    }
}
pub struct Guard {
    access: Box<dyn Access>,
    originals: Vec<(usize, u32, u64)>,
    restore: bool,
}
impl Guard {
    #[cfg(any(target_os = "linux", test))]
    fn apply(
        mut access: Box<dyn Access>,
        cpus: &[usize],
        entries: &[Entry],
        restore: bool,
    ) -> Result<Self> {
        let mut originals = Vec::new();
        // Snapshot all registers before modifying any CPU.
        for &cpu in cpus {
            for entry in entries {
                originals.push((cpu, entry.register, access.read(cpu, entry.register)?));
            }
        }
        let mut guard = Self {
            access,
            originals,
            restore: true,
        };
        for &(cpu, register, original) in &guard.originals {
            let entry = entries.iter().find(|e| e.register == register).unwrap();
            guard.access.write(
                cpu,
                register,
                (original & !entry.mask) | (entry.value & entry.mask),
            )?;
        }
        guard.restore = restore;
        Ok(guard)
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if self.restore {
            for &(cpu, register, value) in self.originals.iter().rev() {
                if let Err(error) = self.access.write(cpu, register, value) {
                    crate::status!(
                        "[msr] restore failed on CPU {cpu}, register {register:#x}: {error:#}"
                    );
                }
            }
        }
    }
}
#[cfg(target_os = "linux")]
pub fn apply(cpus: &[usize], entries: &[Entry], restore: bool) -> Result<Option<Guard>> {
    if entries.is_empty() {
        return Ok(None);
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open("/run/lock/rxminer-msr.lock")
        .context("open MSR lock")?;
    // SAFETY: flock operates on this owned descriptor and does not access Rust memory.
    ensure!(
        // SAFETY: the descriptor is owned by lock and remains open.
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another RxMiner process owns MSR settings"
    );
    let mut files = BTreeMap::new();
    for &cpu in cpus {
        files.insert(
            cpu,
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/cpu/{cpu}/msr"))
                .context("open MSR device (load the msr module and grant access)")?,
        );
    }
    Guard::apply(
        Box::new(Device { files, _lock: lock }),
        cpus,
        entries,
        restore,
    )
    .map(Some)
}

#[cfg(not(target_os = "linux"))]
pub fn apply(_cpus: &[usize], entries: &[Entry], _restore: bool) -> Result<Option<Guard>> {
    ensure!(
        entries.is_empty(),
        "MSR writes require the Linux MSR backend"
    );
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Mock {
        values: Arc<Mutex<Vec<u64>>>,
        fail: bool,
    }
    impl Access for Mock {
        fn read(&mut self, cpu: usize, _: u32) -> Result<u64> {
            Ok(self.values.lock().unwrap()[cpu])
        }
        fn write(&mut self, cpu: usize, _: u32, value: u64) -> Result<()> {
            if self.fail && cpu == 1 {
                self.fail = false;
                bail!("injected write failure");
            }
            self.values.lock().unwrap()[cpu] = value;
            Ok(())
        }
    }
    #[test]
    fn masked_writes_restore_after_exit_and_partial_failure() {
        let entries = parse(&serde_json::json!(["0x1a4:0x5:0xf"])).unwrap();
        for fail in [false, true] {
            let values = Arc::new(Mutex::new(vec![0xa0, 0xb0]));
            let result = Guard::apply(
                Box::new(Mock {
                    values: values.clone(),
                    fail,
                }),
                &[0, 1],
                &entries,
                true,
            );
            if fail {
                assert!(result.is_err());
            } else {
                assert_eq!(*values.lock().unwrap(), [0xa5, 0xb5]);
            }
            drop(result);
            assert_eq!(*values.lock().unwrap(), [0xa0, 0xb0]);
        }
        for bad in ["x:3", "0x100000000:1", "1:2:0", "1"] {
            assert!(entry(bad).is_err());
        }
    }
}
