use std::collections::BTreeMap;

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

#[cfg(test)]
mod tests {
    use super::*;
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
    }
}
