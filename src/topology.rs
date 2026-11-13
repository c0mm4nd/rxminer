//! Miner-owned placement policy over the reusable platform crate.
pub use rx_platform::{
    affinity, available_memory, bind_node, cpu_quota, pin, select, set_affinity, set_priority,
    summary, Core,
};
use std::collections::BTreeSet;

pub fn discover() -> anyhow::Result<Vec<Core>> {
    rx_platform::discover_with_warning(|error| {
        crate::status!("physical CPU discovery unavailable: {error}; using OS scheduling");
    })
}

// Estimate dataset replication overhead before enabling NUMA placement.
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
    fn dataset_replication_respects_memory_headroom() {
        let cores: Vec<_> = (0..8)
            .map(|cpu| Core {
                cpu,
                pinned: true,
                node: cpu % 2,
                smt: cpu >= 4,
            })
            .collect();
        assert!(!use_numa(&cores, true, Some(0)));
        assert!(use_numa(&cores, true, None));
        let required = (2 * 2336 + 8 * 2 + 512) * 1024 * 1024;
        assert!(!use_numa(&cores, true, Some(required - 1)));
        assert!(use_numa(&cores, true, Some(required)));
        assert!(!use_numa(&cores, false, None));
        assert!(!use_numa(&cores[..1], true, None));
    }
}
