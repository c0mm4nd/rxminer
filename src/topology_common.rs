use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Core {
    pub cpu: usize,
    pub pinned: bool,
    pub node: usize,
    pub smt: bool,
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
