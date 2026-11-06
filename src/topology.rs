use anyhow::{bail, Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Core {
    pub cpu: usize,
    pub pinned: bool,
    pub node: usize,
    pub smt: bool,
}

fn cpu_list(text: &str) -> Vec<usize> {
    let mut cpus = BTreeSet::new();
    for entry in text.trim().split(',') {
        if let Some((start, end)) = entry.split_once('-') {
            if let (Ok(start), Ok(end)) = (start.parse::<usize>(), end.parse::<usize>()) {
                if start <= end && end < 1 << 20 {
                    cpus.extend(start..=end);
                }
            }
        } else if let Ok(cpu) = entry.parse::<usize>() {
            if cpu < 1 << 20 {
                cpus.insert(cpu);
            }
        }
    }
    cpus.into_iter().collect()
}

pub fn affinity() -> io::Result<Vec<usize>> {
    for words in [16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384] {
        let mut mask = vec![0usize; words];
        // SAFETY: aligned allocation contains exactly the byte length passed to Linux.
        let result = unsafe {
            libc::sched_getaffinity(
                0,
                std::mem::size_of_val(mask.as_slice()),
                mask.as_mut_ptr().cast(),
            )
        };
        if result == 0 {
            return Ok((0..words * usize::BITS as usize)
                .filter(|cpu| {
                    mask[cpu / usize::BITS as usize] & (1usize << (cpu % usize::BITS as usize)) != 0
                })
                .collect());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINVAL) {
            return Err(error);
        }
    }
    Err(io::Error::other("CPU affinity mask is too large"))
}

pub fn set_priority(priority: Option<u8>, yield_cpu: bool) -> io::Result<()> {
    if let Some(level) = priority {
        let nice = match level {
            0 => 19,
            1 => 10,
            2 => 0,
            3 => -5,
            4 => -10,
            5 => -15,
            _ => return Err(io::Error::other("invalid CPU priority")),
        };
        // SAFETY: Linux applies this numeric nice value to the calling thread.
        if unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, nice) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    if yield_cpu && priority.is_none() {
        let parameter = libc::sched_param { sched_priority: 0 };
        // SAFETY: valid sched_param, zero selects this thread; SCHED_BATCH has no realtime priority.
        if unsafe { libc::sched_setscheduler(0, libc::SCHED_BATCH, &parameter) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub fn pin(cpu: usize) -> io::Result<()> {
    set_affinity(&[cpu])
}

pub fn set_affinity(cpus: &[usize]) -> io::Result<()> {
    let max = *cpus
        .iter()
        .max()
        .ok_or_else(|| io::Error::other("empty CPU mask"))?;
    if max >= 1 << 20 {
        return Err(io::Error::other("CPU index is too large"));
    }
    let bits = usize::BITS as usize;
    let mut mask = vec![0usize; max / bits + 1];
    for &cpu in cpus {
        mask[cpu / bits] |= 1usize << (cpu % bits);
    }
    // SAFETY: Linux reads the mask only for this call, within its allocated length.
    if unsafe {
        libc::sched_setaffinity(
            0,
            std::mem::size_of_val(mask.as_slice()),
            mask.as_ptr().cast(),
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn bind_node(node: usize) -> io::Result<()> {
    if node >= 1 << 20 {
        return Err(io::Error::other("NUMA node index is too large"));
    }
    let bits = usize::BITS as usize;
    let mut mask = vec![0usize; node / bits + 1];
    mask[node / bits] |= 1usize << (node % bits);
    // SAFETY: the kernel reads a valid nodemask; MPOL_BIND is 2 on Linux.
    let result =
        unsafe { libc::syscall(libc::SYS_set_mempolicy, 2, mask.as_ptr(), mask.len() * bits) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn discover() -> Result<Vec<Core>> {
    let allowed: BTreeSet<_> = affinity()
        .context("read allowed CPUs")?
        .into_iter()
        .collect();
    let online = fs::read_to_string("/sys/devices/system/cpu/online")
        .map(|s| cpu_list(&s))
        .unwrap_or_else(|_| allowed.iter().copied().collect());
    let mut nodes = BTreeMap::new();
    let node_list = fs::read_to_string("/sys/devices/system/node/online").unwrap_or_default();
    for node in cpu_list(&node_list) {
        if let Ok(list) = fs::read_to_string(format!("/sys/devices/system/node/node{node}/cpulist"))
        {
            for cpu in cpu_list(&list) {
                nodes.insert(cpu, node);
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut cores = Vec::new();
    for cpu in online.into_iter().filter(|cpu| allowed.contains(cpu)) {
        let siblings = fs::read_to_string(format!(
            "/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list"
        ))
        .map(|s| cpu_list(&s))
        .unwrap_or_default();
        let physical = siblings.first().copied().unwrap_or(cpu);
        cores.push(Core {
            cpu,
            pinned: true,
            node: *nodes.get(&cpu).unwrap_or(&0),
            smt: !seen.insert(physical),
        });
    }
    if cores.is_empty() {
        bail!("no usable CPU cores found");
    }
    Ok(cores)
}

pub fn select(all: &[Core], requested: usize, smt: bool) -> Vec<Core> {
    let capacity = all.iter().filter(|c| smt || !c.smt).count();
    let count = if requested == 0 {
        capacity
    } else {
        requested.min(capacity)
    };
    let mut selected = Vec::with_capacity(count);
    for use_smt in [false, true].into_iter().take(if smt { 2 } else { 1 }) {
        let mut groups: BTreeMap<usize, Vec<&Core>> = BTreeMap::new();
        for core in all.iter().filter(|c| c.smt == use_smt) {
            groups.entry(core.node).or_default().push(core);
        }
        let rounds = groups.values().map(Vec::len).max().unwrap_or(0);
        for round in 0..rounds {
            for group in groups.values() {
                if selected.len() == count {
                    break;
                }
                if let Some(core) = group.get(round) {
                    selected.push((*core).clone());
                }
            }
            if selected.len() == count {
                break;
            }
        }
    }
    selected.sort_by_key(|c| (c.node, c.cpu));
    selected
}

pub fn summary(cores: &[Core]) -> String {
    let mut nodes = BTreeMap::new();
    for core in cores {
        *nodes.entry(core.node).or_insert(0) += 1;
    }
    format!(
        "{} threads ({})",
        cores.len(),
        nodes
            .into_iter()
            .map(|(n, c)| format!("node{n}:{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// Locate the current process's controller and all visible ancestors. Walking
// ancestors matters for systemd slices and containers with an inherited quota.
fn cgroup_dirs(controller: &str) -> Vec<PathBuf> {
    let membership = fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    let mounts = fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let mut dirs = BTreeSet::new();
    for line in membership.lines() {
        let parts: Vec<_> = line.splitn(3, ':').collect();
        if parts.len() != 3 {
            continue;
        }
        let unified = parts[1].is_empty();
        if !unified && !parts[1].split(',').any(|c| c == controller) {
            continue;
        }
        for mount in mounts.lines() {
            let Some((before, after)) = mount.split_once(" - ") else {
                continue;
            };
            let pre: Vec<_> = before.split_whitespace().collect();
            let post: Vec<_> = after.split_whitespace().collect();
            if pre.len() < 5 || post.len() < 3 {
                continue;
            }
            if (unified && post[0] != "cgroup2")
                || (!unified
                    && (post[0] != "cgroup" || !post[2].split(',').any(|c| c == controller)))
            {
                continue;
            }
            let root = PathBuf::from(unescape_mount(pre[3]));
            let mountpoint = PathBuf::from(unescape_mount(pre[4]));
            let path = Path::new(parts[2]);
            let relative = if let Ok(relative) = path.strip_prefix(&root) {
                relative
            } else if path == Path::new("/") {
                Path::new("")
            } else {
                continue;
            };
            // Ignore namespace paths that would escape the visible mount.
            if relative
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                continue;
            }
            let mut current = mountpoint.join(relative);
            loop {
                dirs.insert(current.clone());
                if current == mountpoint || !current.pop() {
                    break;
                }
            }
        }
    }
    dirs.insert(PathBuf::from("/sys/fs/cgroup"));
    dirs.insert(PathBuf::from(format!("/sys/fs/cgroup/{controller}")));
    dirs.into_iter().collect()
}

fn unescape_mount(text: &str) -> String {
    text.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn quota(quota: &str, period: &str) -> Option<usize> {
    let quota: u64 = quota.trim().parse().ok()?;
    let period: u64 = period.trim().parse().ok()?;
    if quota == 0 || period == 0 {
        return None;
    }
    usize::try_from(quota.div_ceil(period)).ok()
}

pub fn cpu_quota() -> Option<usize> {
    cgroup_dirs("cpu")
        .into_iter()
        .filter_map(|dir| {
            if let Ok(value) = fs::read_to_string(dir.join("cpu.max")) {
                let mut fields = value.split_whitespace();
                quota(fields.next()?, fields.next()?)
            } else {
                quota(
                    &fs::read_to_string(dir.join("cpu.cfs_quota_us")).ok()?,
                    &fs::read_to_string(dir.join("cpu.cfs_period_us")).ok()?,
                )
            }
        })
        .min()
}

fn headroom(limit: &str, usage: &str) -> Option<u64> {
    let limit: u64 = limit.trim().parse().ok()?;
    let usage: u64 = usage.trim().parse().ok()?;
    if limit > 1 << 60 {
        return None;
    }
    Some(limit.saturating_sub(usage))
}

fn host_available(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        if words.next()? != "MemAvailable:" {
            return None;
        }
        words.next()?.parse::<u64>().ok()?.checked_mul(1024)
    })
}

pub fn available_memory() -> Option<u64> {
    let host = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| host_available(&s));
    cgroup_dirs("memory")
        .into_iter()
        .filter_map(|dir| {
            [
                ("memory.max", "memory.current"),
                ("memory.limit_in_bytes", "memory.usage_in_bytes"),
            ]
            .into_iter()
            .find_map(|(limit, usage)| {
                headroom(
                    &fs::read_to_string(dir.join(limit)).ok()?,
                    &fs::read_to_string(dir.join(usage)).ok()?,
                )
            })
        })
        .chain(host)
        .min()
}

pub fn use_numa(cores: &[Core], requested: bool, available: Option<u64>) -> bool {
    let nodes = cores.iter().map(|c| c.node).collect::<BTreeSet<_>>().len();
    if !requested || nodes <= 1 {
        return false;
    }
    let required = (nodes as u64 * 2336 + cores.len() as u64 * 2 + 512) * 1024 * 1024;
    if let Some(available) = available.filter(|bytes| *bytes < required) {
        crate::status!(
            "memory-aware NUMA: {} MiB available, {} MiB needed; using one dataset",
            available / 1048576,
            required / 1048576
        );
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_cpu_lists() {
        assert_eq!(cpu_list("0-2,8,32-33"), [0, 1, 2, 8, 32, 33]);
    }
    #[test]
    fn memory_zero_is_not_unknown() {
        assert_eq!(headroom("4096", "4096"), Some(0));
        assert_eq!(headroom("4096", "5000"), Some(0));
        assert_eq!(headroom("4096", "1024"), Some(3072));
        for limit in ["max", "-1", "9223372036854771712", "4096junk"] {
            assert_eq!(headroom(limit, "0"), None);
        }
        assert_eq!(host_available("MemAvailable: 0 kB\n"), Some(0));
        assert_eq!(quota("150000", "100000"), Some(2));
        assert_eq!(quota("max", "100000"), None);
    }
    #[test]
    fn selection_balances_nodes_before_smt() {
        let all: Vec<_> = (0..8)
            .map(|cpu| Core {
                cpu,
                pinned: true,
                node: cpu % 2,
                smt: cpu >= 4,
            })
            .collect();
        assert_eq!(select(&all, 0, false).len(), 4);
        assert_eq!(select(&all, 99, true).len(), 8);
        let two = select(&all, 2, false);
        assert_ne!(two[0].node, two[1].node);
        assert_eq!(select(&all, 5, true).iter().filter(|c| c.smt).count(), 1);
        assert!(!use_numa(&all, true, Some(0)));
        assert!(use_numa(&all, true, None));
    }
    #[test]
    fn restricted_sibling_remains_a_usable_core() {
        // Affinity is per thread. This dedicated thread exits without changing
        // the test harness's or the caller's affinity.
        std::thread::spawn(|| {
            let all = discover().unwrap();
            let chosen = all
                .iter()
                .find(|c| c.smt)
                .unwrap_or_else(|| all.last().unwrap());
            pin(chosen.cpu).unwrap();
            let restricted = discover().unwrap();
            assert_eq!(restricted.len(), 1);
            assert!(!restricted[0].smt);
            assert_eq!(select(&restricted, 0, false).len(), 1);
        })
        .join()
        .unwrap();
    }
}
