#!/usr/bin/env python3
"""Compare two local miner binaries using externally sampled hash counters.

Runs baseline / candidate / candidate / baseline to reduce first-order time drift. Initialization
and warm-up are excluded from steady-state rates. No pool traffic is generated.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import statistics
import subprocess
import time
import urllib.error
import urllib.request


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def memory(pid):
    result = {}
    try:
        for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines():
            key, _, value = line.partition(":")
            if key in ("Rss", "AnonHugePages", "Private_Hugetlb", "Shared_Hugetlb"):
                result[key + "_kib"] = int(value.split()[0])
    except (FileNotFoundError, PermissionError):
        pass
    return result


def run_one(label, binary, args, index, opener):
    port = free_port()
    path = args.output.parent / f"{args.output.stem}-{index}-{label}.log"
    duration = int(args.warmup + args.seconds + 30)
    command = ["taskset", "-c", args.cpus, binary, "--benchmark", str(duration),
               "--threads", str(args.threads), "--http-port", str(port)]
    if args.no_numa:
        command.append("--no-numa")
    env = dict(os.environ)
    env.pop("RXM_THP", None)
    env.pop("RXM_COLLAPSE", None)
    if args.no_thp:
        env["RXM_THP"] = "0"
    print(f"START {index} {label}: {args.threads} threads, warmup {args.warmup}s, measure {args.seconds}s", flush=True)
    started = time.monotonic()
    with path.open("w") as log:
        child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=env)
        try:
            def sample():
                if child.poll() is not None:
                    raise RuntimeError(f"miner exited unexpectedly: {path.read_text()}")
                before = time.monotonic()
                with opener.open(f"http://127.0.0.1:{port}/summary", timeout=5) as response:
                    data = json.load(response)
                after = time.monotonic()
                return {"time": (before + after) / 2, "request_seconds": after - before,
                        "hashes": data["hashes"], "threads": data["threads"], "nodes": data["nodes"]}

            deadline = started + 240
            while True:
                try:
                    ready = sample()
                    if ready["hashes"] > 0:
                        break
                except (OSError, urllib.error.URLError):
                    pass
                if time.monotonic() > deadline:
                    raise TimeoutError("dataset initialization timed out")
                time.sleep(0.1)
            startup = ready["time"] - started
            assert ready["threads"] == args.threads, ready
            print(f"READY {index} {label}: {startup:.3f}s, {ready['nodes']} datasets", flush=True)
            time.sleep(args.warmup)
            first = sample()
            samples = [first]
            while time.monotonic() < first["time"] + args.seconds:
                remaining = first["time"] + args.seconds - time.monotonic()
                time.sleep(max(0, min(5, remaining)))
                samples.append(sample())
            last = samples[-1]
            elapsed = last["time"] - first["time"]
            hashes = last["hashes"] - first["hashes"]
            result = {"label": label, "index": index, "binary": binary,
                      "binary_sha256": sha256(binary), "command": command,
                      "startup_seconds": startup, "nodes": ready["nodes"],
                      "measurement_seconds": elapsed, "hashes": hashes,
                      "hashes_per_second": hashes / elapsed, "samples": samples,
                      "memory": memory(child.pid), "loadavg": Path("/proc/loadavg").read_text().strip()}
            print(f"DONE {index} {label}: {result['hashes_per_second']:.2f} H/s, {result['memory']}", flush=True)
            return result
        finally:
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
                try:
                    child.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", default="build-cpp-baseline/build/rxminer")
    parser.add_argument("--candidate", default="target/release/rxminer")
    parser.add_argument("--baseline-revision", help="source revision used to build the baseline")
    parser.add_argument("--cpus", required=True)
    parser.add_argument("--threads", type=int, required=True)
    parser.add_argument("--warmup", type=float, default=15)
    parser.add_argument("--seconds", type=float, default=45)
    parser.add_argument("--no-numa", action="store_true")
    parser.add_argument("--no-thp", action="store_true",
                        help="disable THP hints in both binaries to control page-backing variation")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    assert args.threads > 0 and args.seconds > 0 and args.warmup >= 0
    args.output.parent.mkdir(parents=True, exist_ok=True)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    result = {"utc_started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "kernel": os.uname().release, "cpu_model": next(line.split(":", 1)[1].strip()
                  for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")),
              "baseline_revision": args.baseline_revision,
              "thp": Path("/sys/kernel/mm/transparent_hugepage/enabled").read_text().strip(),
              "thp_hint_enabled": not args.no_thp, "forced_collapse": False,
              "cpus": args.cpus, "threads": args.threads, "warmup_seconds": args.warmup,
              "measurement_target_seconds": args.seconds, "runs": []}
    for index, (label, binary) in enumerate([("baseline", args.baseline), ("candidate", args.candidate), ("candidate", args.candidate), ("baseline", args.baseline)], 1):
        result["runs"].append(run_one(label, binary, args, index, opener))
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    means = {label: statistics.mean(run["hashes_per_second"] for run in result["runs"] if run["label"] == label)
             for label in ("baseline", "candidate")}
    result["mean_hashes_per_second"] = means
    result["candidate_vs_baseline_percent"] = (means["candidate"] / means["baseline"] - 1) * 100
    startup = {label: statistics.mean(run["startup_seconds"] for run in result["runs"] if run["label"] == label)
               for label in ("baseline", "candidate")}
    result["mean_startup_seconds"] = startup
    result["startup_time_reduction_percent"] = (1 - startup["candidate"] / startup["baseline"]) * 100
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"mean_hashes_per_second": means, "candidate_vs_baseline_percent": result["candidate_vs_baseline_percent"], "mean_startup_seconds": startup, "startup_time_reduction_percent": result["startup_time_reduction_percent"]}), flush=True)


if __name__ == "__main__":
    main()
