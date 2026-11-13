# rx-platform

Rust CPU topology, worker selection, affinity, priorities, NUMA binding and
available CPU/memory queries, independent of RandomX and the miner.

```rust,no_run
let cores = rx_platform::discover_with_warning(|error| eprintln!("{error}"))?;
let workers = rx_platform::select(&cores, 0, false);
println!("{}", rx_platform::summary(&workers));
# Ok::<(), anyhow::Error>(())
```

- Linux: permitted physical CPUs and SMT siblings, NUMA nodes, thread affinity,
  memory binding, cgroup v1/v2 CPU quotas and available-memory headroom.
- Windows: physical-core discovery within the current processor group, affinity,
  thread priority and available physical memory. Discovery failures fall back to
  unbound workers; `discover_with_warning` exposes the diagnostic to the caller.
- macOS: physical/logical worker counts and reclaimable memory. Worker IDs are
  slots; `Core::pinned` is false and hard affinity is unsupported.

`select` balances physical cores across nodes before adding SMT siblings.
`available_memory` returns `None` when unknown; `Some(0)` means no headroom.
OS errors propagate from binding/priority calls. These calls affect the current
thread or process as supported by the OS; discovering CPUs does not bind them.
RandomX dataset sizing, replication policy, MSR tuning and logging stay in the
miner. No build script, C/C++ build dependency or backend feature is required.

Run `cargo test -p rx-platform --release` explicitly when testing the workspace.
