#!/usr/bin/env python3
"""Compare shutdown latency while local TLS/proxy peers stall their handshakes.

No pool login completes and no mining dataset is allocated. All sockets are
loopback-only. Both user and developer connections must reach the stalled peer
before SIGINT, so measurements exclude connection setup.
"""
import argparse
import hashlib
import json
from pathlib import Path
import signal
import socket
import statistics
import subprocess
import tempfile
import time


def run_one(binary, label, mode):
    binary = str(Path(binary).resolve())
    peers = []
    with socket.socket() as listener, tempfile.TemporaryFile(mode="w+") as log:
        listener.bind(("127.0.0.1", 0))
        listener.listen(2)
        listener.settimeout(10)
        address = f"127.0.0.1:{listener.getsockname()[1]}"
        command = [binary, "-o", f"stratum+ssl://{address}", "-u", "test-wallet",
                   "--threads", "1", "--no-proxy"]
        if mode == "http-proxy":
            command += ["--proxy", f"http://{address}"]
        child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
        try:
            for _ in range(2):
                peer, _ = listener.accept()
                peers.append(peer)
                peer.settimeout(10)
                if not peer.recv(2048):
                    raise RuntimeError("client disconnected before handshake")
            started = time.monotonic()
            child.send_signal(signal.SIGINT)
            code = child.wait(timeout=12)
            elapsed = time.monotonic() - started
            if code != 0:
                log.seek(0)
                raise RuntimeError(f"shutdown exited {code}: {log.read()}")
            result = {"label": label, "mode": mode, "binary": binary,
                      "sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
                      "command": command, "shutdown_seconds": elapsed, "exit_code": code}
            print(f"{mode} {label}: {elapsed:.4f}s", flush=True)
            return result
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            for peer in peers:
                peer.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--candidate", default="target/release/rxminer")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    result = {"utc_started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "method": "SIGINT after both local peers receive handshake bytes; ABBA per mode",
              "runs": []}
    for mode in ("tls", "http-proxy"):
        for label, binary in [("baseline", args.baseline), ("candidate", args.candidate),
                              ("candidate", args.candidate), ("baseline", args.baseline)]:
            result["runs"].append(run_one(binary, label, mode))
            args.output.write_text(json.dumps(result, indent=2) + "\n")
    result["mean_shutdown_seconds"] = {
        mode: {label: statistics.mean(r["shutdown_seconds"] for r in result["runs"]
                                       if r["mode"] == mode and r["label"] == label)
               for label in ("baseline", "candidate")} for mode in ("tls", "http-proxy")
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["mean_shutdown_seconds"]), flush=True)


if __name__ == "__main__":
    main()
