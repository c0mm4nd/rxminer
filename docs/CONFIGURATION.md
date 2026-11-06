# Configuration

RxMiner uses familiar `pools`, `cpu`, `randomx`, `http`, and logging options.
CLI values override JSON. Start from [config.example.json](../config.example.json)
and validate with `rxminer -c config.json --dry-run`. Validation does not connect
to pools, allocate datasets, or write MSRs. TLS server certificates are checked
when configured. Mining uses `rx/0`; see [platform support](PLATFORMS.md) for OS-specific capabilities.

Option names follow the [CLI reference](https://xmrig.com/docs/miner/command-line-options)
and [CPU configuration reference](https://xmrig.com/docs/miner/config/cpu).
Unsupported settings are rejected; accepting a configuration does not imply
identical automatic CPU selection or identical API responses in other miners.

## Pools and networking

| CLI | JSON | Behavior |
| --- | --- | --- |
| `-o`, `--url` | `pools[].url` | Host:port or Stratum URL; default port 3333 |
| `-a`, `--algo` | `pools[].algo` | `rx/0`, with `rx` alias |
| `--coin` | `pools[].coin` | `monero` or `xmr` |
| `-u`, `--user` | `pools[].user` | Username or wallet |
| `-p`, `--pass` | `pools[].pass` | Password; default `x` |
| `-O`, `--userpass` | User and password | `USER:PASS`; excludes separate `-u`/`-p` |
| `--rig-id` | `pools[].rig-id` | Optional worker identifier |
| `--tls` | `pools[].tls` | TLS to the pool |
| `--tls-fingerprint` | `pools[].tls-fingerprint` | SHA-256 certificate pin |
| `-x`, `--proxy` | `pools[].socks5` | SOCKS5; bare host:port uses remote DNS |
| `--nicehash` | `pools[].nicehash` | Preserve the nonce prefix and enumerate the lower 24 bits |
| `-k`, `--keepalive` | `pools[].keepalive` | `true` sends every 60 seconds; JSON also accepts seconds; `false` disables |
| `-r`, `--retries` | `retries` | Attempts before switching pools; default 5 |
| `-R`, `--retry-pause` | `retry-pause` | Seconds between attempts; default 5 |
| `--user-agent` | `user-agent` | Login agent string |

The first enabled pool is primary; other enabled entries are backups. All
workers and connections stop before another pool starts. A single-pool developer
connection failure remains fatal; with backups, mining stops while reconnecting
or switching. The built-in fee is retained on each selected pool.
NiceHash support can also be negotiated by the login response.

`dns.ip_version` accepts 0 (both), 4 or 6; `dns.ttl` caches successful local
lookups for that many seconds (default 30, zero disables caching). A bounded
resolver worker allows callers to stop while a system lookup is pending.
SOCKS5h and HTTP CONNECT leave target-name resolution to the proxy.

`--no-proxy` disables environment proxy discovery. Explicit proxy settings take
precedence. `--proxy http://...` supports HTTP CONNECT. `--tls-verify`, `--tls-ca`
and matching pool fields are client-trust extensions. The root `tls` section
configures the API server, separately from pool TLS.

## CPU and memory

| CLI | JSON | Behavior |
| --- | --- | --- |
| `-t`, `--threads` | `cpu.rx` profile | Explicit worker count; zero selects automatically |
| `--cpu-affinity` | Profile affinity | Decimal or hexadecimal CPU mask |
| `--cpu-max-threads-hint` | `cpu.max-threads-hint` | Percentage hint for automatic selection |
| `--cpu-priority` | `cpu.priority` | 0..5; priorities above 2 require scheduling privileges |
| `--cpu-no-yield` | `cpu.yield: false` | Disable the default batch-scheduling preference |
| `--no-huge-pages` | `cpu.huge-pages: false` | Disable reserved 2 MiB scratchpad/dataset pages |
| `--hugepage-size 2048` | `cpu.huge-pages: 2048` | Explicit page size in KiB |
| `--huge-pages-jit` | `cpu.huge-pages-jit` | Request reserved pages for JIT buffers |
| `--randomx-1gb-pages` | `randomx.1gb-pages` | Request 1 GiB dataset pages; fall back to smaller pages |
| `--randomx-mode` | `randomx.mode` | `fast`: dataset; `light`: shared cache; `auto`: dataset with memory fallback |
| `--randomx-init` | `randomx.init` | Initialization threads per dataset; -1/0 selects automatically |
| `--randomx-no-numa` | `randomx.numa: false` | Share one dataset/cache |
| `--argon2-impl` | `cpu.argon2-impl` | `auto`, `x86_64`, `SSSE3`, `AVX2`; CPU support is checked |

`cpu.hw-aes` is `null` for detection or a boolean override. Hardware AES cannot
be forced on an unsupported CPU. `cpu.memory-pool` is false, true (one retained
2 MiB scratchpad per worker), or a page count. Idle scratchpads are reused only
for the same NUMA node and allocation type; each live VM has exclusive ownership.
The pool is cleared when runtime configuration is reapplied.

`randomx.scratchpad_prefetch_mode`: 0 disables prefetch, 1 uses T0 (default),
2 uses NTA, and 3 uses a load. JIT instances retain their selected mode and mapping
length. Reserved pages must already be available; the miner does not reserve
system pages. A 1 GiB dataset mapping requires three pages. Logs distinguish
2 MiB datasets from 1 GiB datasets. JIT huge-page allocation falls back silently.

Profiles select `cpu["rx/0"]`, then `cpu.rx`, then `cpu["*"]`. Entries can be
CPU IDs (`-1` means unbound), `[1, CPU_ID]` pairs, or an object containing
`threads`, `intensity: 1` and `affinity`. Profile names can reference other profiles:

```json
{"cpu":{"rx":"small","small":[-1,-1]}}
```

Unknown references and cycles fail validation. Explicit CPU IDs must be in the
process's allowed set. `-t N` overrides the profile with N unbound workers unless
an affinity mask is supplied. `--smt`/`cpu.smt` and `--no-thp`/`cpu.thp` remain
extensions. `--no-numa` is an alias for `--randomx-no-numa`.

## MSRs

MSR writes are opt-in: `randomx.wrmsr` defaults to false. `true` selects a known
CPU preset; unsupported CPU models require explicit register definitions.
Intel also accepts a numeric prefetch value from 0 to 15. Custom entries use
`"register:value[:mask]"`, with decimal or hexadecimal integers. Register
presets follow the [MSR reference](https://xmrig.com/docs/miner/randomx-optimization-guide/msr).

`randomx.rdmsr` defaults to true and restores original values after workers
stop. False explicitly leaves successful changes applied. Partial setup failures
always roll back. RxMiner takes an exclusive process lock and snapshots all
selected CPUs before writing; device access must already be available.
It does not load kernel modules. Forced process termination cannot run cleanup.

## Runtime and output

`autosave` defaults to true and writes a generated CPU profile to the file passed
with `-c`. `watch` defaults to true; `--no-watch` disables it. Writes are atomic.
Reload first validates the configuration and CPU IDs, then stops and joins old
workers before starting replacements. Invalid edits keep the old configuration.
Changing background mode requires a process restart.

`-B`/`--background`, `-l`/`--log-file`, `-S`/`--syslog`, `--no-color`, and
`--verbose` map to `background`, `log-file`, `syslog`, `colors`, and `verbose`.
Log files append; syslog uses the local Unix socket.
`print-time` controls speed output; `health-print-time` controls available-memory
and worker-count output. Both default to 60 seconds; zero disables periodic output.
`pause-on-battery`/`--pause-on-battery` pauses mining when Linux reports a
discharging battery without an online external power supply. Pauses freeze the
fee schedule and preserve nonce progress.

## HTTP API

Enable with `http.enabled` plus a nonzero `http.port`, or `--http-port`.
`--http-host` and `--http-access-token` override the corresponding fields.
Tokens use `Authorization: Bearer TOKEN`. Authentication covers every endpoint.

Read-only endpoints: `/`, `/summary`, `/hashrates`, `/shares`. `/1/summary` and
`/2/summary` expose common summary fields and a three-window hashrate array;
they do not implement the entire external API schema.

`http.restricted: false` requires a token and enables:

- `POST /json_rpc`: `pause`, `resume`, `stop`. Pause/resume apply to mining.
- `GET /1/config` or `/2/config`: read the source configuration, including secrets.
- `PUT /1/config` or `/2/config`: validate and atomically save the replacement;
  restart workers even when file watching is disabled. Requires `-c`.

Root `tls` accepts `enabled`, `cert`, `cert_key`, `protocols`, `ciphers`,
`ciphersuites`, and `dhparam`. Certificates and keys use PEM. Allowed protocols
are `TLSv1.2` and/or `TLSv1.3`, separated by spaces. API requests have bounded
headers, bodies and read deadlines; ambiguous framing is rejected.

## Offline benchmarks

`--bench 250K`, `500K`, or `1M`..`10M` runs an exact hash count. JSON uses
`benchmark.size`. `--seed`/`benchmark.seed` takes a 32-byte hex seed.
`--hash`/`benchmark.hash` verifies a 16-digit hexadecimal checksum: the XOR of
the first little-endian 64-bit word of every hash. It is independent of worker
count for a fixed input and seed. RxMiner uses its own benchmark input; checksums
are not interchangeable with other miners' benchmark results.

`--benchmark-seconds N` (legacy `--benchmark N`) and `benchmark.seconds` run a
timed test; checksum verification requires an exact-count test. Benchmarks have
no pool connection or developer fee. Online `submit` and remote-ID `verify`
options are removed, with no calls to an external benchmark service.

## Remaining implementation work

These features are still unavailable and must not be treated as implemented:

- GPU backends, RISC-V and Windows ARM64 validation, and algorithms other than `rx/0`.
- Non-Linux NUMA placement and physical-core discovery; see [platform support](PLATFORMS.md).
- Daemon RPC mining and self-select/origin submission.
- Cache QoS, named assembly kernels, and SSE2/XOP/AVX-512 Argon2 kernels.
- AVX2 dataset initialization (`randomx.init-avx2: 1`); selecting an AVX2 Argon2
  cache kernel is a different setting. Values -1/0 retain the current initializer.
- Activity-based pausing and the complete external HTTP API schema.

The developer fee remains fixed and unchanged. `donate-level` and
`donate-over-proxy` are not accepted. No external donation endpoint is introduced.
