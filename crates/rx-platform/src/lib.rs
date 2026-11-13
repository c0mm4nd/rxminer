//! CPU discovery, worker selection, affinity and operating-system resource limits.
//!
//! Linux supports NUMA placement and cgroup limits. Windows discovers CPUs within
//! the current processor group. macOS exposes unbound worker slots, not pinnable
//! CPU IDs. This crate has no dependency on a mining algorithm or logging system.
#[cfg(target_os = "linux")]
mod topology;
#[cfg(not(target_os = "linux"))]
#[path = "topology_portable.rs"]
mod topology;
mod topology_common;

pub use topology::{
    affinity, available_memory, bind_node, cpu_quota, discover, pin, select, set_affinity,
    set_priority, summary, Core,
};

/// Discover workers and report recoverable topology failures to the caller.
///
/// Windows falls back to unbound workers if physical-core discovery fails.
/// Affinity-query failures and an empty CPU set remain errors. The callback is
/// synchronous and is never retained. [`discover`] uses the same fallback but
/// ignores the optional warning.
pub fn discover_with_warning(warning: impl FnMut(&anyhow::Error)) -> anyhow::Result<Vec<Core>> {
    #[cfg(target_os = "linux")]
    {
        let _ = warning;
        topology::discover()
    }
    #[cfg(not(target_os = "linux"))]
    topology::discover_with_warning(warning)
}
