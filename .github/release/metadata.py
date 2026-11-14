"""Record the identity of a production artifact; the extracted smoke checks its backend."""
import argparse
import json
import os
import subprocess

from verify import BACKENDS, TARGETS, require

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--platform", choices=TARGETS, required=True)
parser.add_argument("--backend", choices=BACKENDS, required=True)
parser.add_argument("--version", required=True)
args = parser.parse_args()
rustc = subprocess.check_output(["rustc", "-vV"], text=True)
target = next(line.removeprefix("host: ") for line in rustc.splitlines() if line.startswith("host: "))
require(target in TARGETS[args.platform], "runner architecture does not match release platform")
require(os.environ.get("RXMINER_ARCH", "default") == "default", "release must not use native CPU tuning")
require("native" not in os.environ.get("RUSTFLAGS", "") + os.environ.get("CARGO_ENCODED_RUSTFLAGS", ""),
        "release must not use native Rust CPU tuning")
print(json.dumps({
    "version": args.version,
    "platform": args.platform,
    "backend": args.backend,
    "features": [f"randomx-{args.backend}"],
    "target": target,
    "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "rustc": rustc.splitlines()[0],
    "runner_image": os.environ.get("ImageOS", ""),
}, indent=2))
