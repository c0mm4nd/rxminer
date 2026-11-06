#!/usr/bin/env python3
"""Run consensus/JIT regressions against the exact library built by Cargo (no CMake)."""
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parent.parent
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"], cwd=root,
    ))
    package = next(p for p in metadata["packages"] if p["name"] == "rxminer")
    messages = subprocess.check_output(
        ["cargo", "build", "--lib", "--release", "--locked", "--message-format=json"],
        cwd=root, text=True,
    )
    outputs = [json.loads(line) for line in messages.splitlines()]
    build = next(m for m in outputs if m["reason"] == "build-script-executed"
                 and m["package_id"] == package["id"])
    library = Path(build["out_dir"]) / "librandomx.a"
    source = root / "src/crypto/randomx"
    with tempfile.TemporaryDirectory(prefix="rxminer-randomx-tests-") as temporary:
        for name in ("tests", "jit_optimizations", "program_generation", "blake_fixed"):
            binary = Path(temporary) / name
            command = shlex.split(os.environ.get("CXX", "c++")) + [
                "-std=c++11", "-O2", "-maes", "-DHAVE_POSIX_MEMALIGN", "-I", str(source),
                str(root / "tests/randomx" / f"{name}.cpp"), str(library), "-pthread", "-lm", "-o", str(binary),
            ]
            subprocess.run(command, cwd=root, check=True)
            subprocess.run([binary], cwd=root, check=True)
            if name == "jit_optimizations":
                subprocess.run([binary, "--normalized"], cwd=root, check=True)


if __name__ == "__main__":
    main()
