//! Portable worker selection with explicit platform capability boundaries.
pub use crate::topology_common::{select, summary, Core};
use anyhow::{ensure, Result};
use std::io;

pub fn affinity() -> io::Result<Vec<usize>> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::{SystemInformation::GROUP_AFFINITY, Threading::*};
        let mut group = GROUP_AFFINITY::default();
        let (mut process_mask, mut system_mask) = (0, 0);
        // SAFETY: API calls fill valid stack outputs; pseudo-handles need no close.
        let ok = unsafe {
            GetThreadGroupAffinity(GetCurrentThread(), &mut group) != 0
                && GetProcessAffinityMask(GetCurrentProcess(), &mut process_mask, &mut system_mask)
                    != 0
        };
        if !ok {
            return Err(io::Error::last_os_error());
        }
        Ok((0..usize::BITS as usize)
            .filter(|bit| process_mask & group.Mask & (1usize << bit) != 0)
            .map(|bit| usize::from(group.Group) * 64 + bit)
            .collect())
    }
    #[cfg(not(windows))]
    Ok((0..std::thread::available_parallelism()?.get()).collect())
}

pub fn discover() -> Result<Vec<Core>> {
    discover_with_warning(|_| {})
}

pub(super) fn discover_with_warning(warning: impl FnMut(&anyhow::Error)) -> Result<Vec<Core>> {
    let cpus = affinity()?;
    ensure!(!cpus.is_empty(), "no usable CPUs");
    #[cfg(windows)]
    {
        let mut warning = warning;
        windows::discover(&cpus).or_else(|error| {
            warning(&error);
            Ok(unbound(&cpus, cpus.len()))
        })
    }
    #[cfg(not(windows))]
    let _ = warning;
    #[cfg(target_os = "macos")]
    let physical = macos::physical_cpus().unwrap_or(cpus.len());
    #[cfg(not(any(windows, target_os = "macos")))]
    let physical = cpus.len();
    #[cfg(not(windows))]
    Ok(unbound(&cpus, physical))
}

fn unbound(cpus: &[usize], physical: usize) -> Vec<Core> {
    // On macOS these are worker slots, not hard-bindable CPU identifiers.
    // Count physical cores first without pretending to know SMT sibling IDs.
    cpus.iter()
        .enumerate()
        .map(|(index, &cpu)| Core {
            cpu,
            pinned: false,
            node: 0,
            smt: index >= physical,
        })
        .collect()
}

pub fn pin(cpu: usize) -> io::Result<()> {
    set_affinity(&[cpu])
}
pub fn set_affinity(cpus: &[usize]) -> io::Result<()> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::{SystemInformation::GROUP_AFFINITY, Threading::*};
        let first = *cpus
            .first()
            .ok_or_else(|| io::Error::other("empty CPU mask"))?;
        let group =
            u16::try_from(first / 64).map_err(|_| io::Error::other("CPU group out of range"))?;
        if cpus.iter().any(|cpu| cpu / 64 != usize::from(group)) {
            return Err(io::Error::other(
                "one thread cannot span Windows processor groups",
            ));
        }
        let mask = cpus
            .iter()
            .fold(0usize, |mask, cpu| mask | (1usize << (cpu % 64)));
        let affinity = GROUP_AFFINITY {
            Group: group,
            Mask: mask,
            Reserved: [0; 3],
        };
        // SAFETY: affinity describes a valid group mask; the OS validates availability.
        if unsafe { SetThreadGroupAffinity(GetCurrentThread(), &affinity, std::ptr::null_mut()) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = cpus;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "hard CPU affinity is unavailable on this platform",
        ))
    }
}

pub fn set_priority(priority: Option<u8>, yield_cpu: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::*;
        let level = match priority {
            Some(0) => -15,
            Some(1) => -2,
            Some(2) => 0,
            Some(3) => 1,
            Some(4) => 2,
            Some(5) => 15,
            None => {
                if yield_cpu {
                    -1
                } else {
                    0
                }
            }
            _ => return Err(io::Error::other("invalid priority")),
        };
        // SAFETY: only the current thread's scheduling priority is changed.
        if unsafe { SetThreadPriority(GetCurrentThread(), level) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    #[cfg(unix)]
    {
        let _ = yield_cpu;
        if let Some(level) = priority {
            let nice = [19, 10, 0, -5, -10, -15]
                .get(usize::from(level))
                .copied()
                .ok_or_else(|| io::Error::other("invalid priority"))?;
            // SAFETY: Darwin applies this nice value to the current process.
            if unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, nice) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}
pub fn bind_node(node: usize) -> io::Result<()> {
    if node == 0 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "NUMA memory binding is unavailable on this platform",
        ))
    }
}
pub fn cpu_quota() -> Option<usize> {
    std::thread::available_parallelism().ok().map(usize::from)
}
pub fn available_memory() -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: correctly sized writable MEMORYSTATUSEX structure.
        (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullAvailPhys)
    }
    #[cfg(target_os = "macos")]
    {
        macos::available_memory()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    None // Unknown headroom is not zero or total installed memory.
}

#[cfg(windows)]
#[path = "topology_windows.rs"]
mod windows;

#[cfg(target_os = "macos")]
#[path = "topology_macos.rs"]
mod macos;
