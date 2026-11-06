#!/usr/bin/env python3
"""Verify packaging and a fresh Cargo installation with CMake/Make/Ninja blocked.

The default checks cargo install --path. Add --git for an isolated Git install.
No remote writes or pool traffic; requires native compilers and 4 GiB free RAM.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile


def run(command, *, cwd, env):
    print("+ " + " ".join(map(str, command)), flush=True)
    subprocess.run(command, cwd=cwd, env=env, check=True)


def check_binary(binary, *, cwd, env):
    run([str(binary), "--version"], cwd=cwd, env=env)
    subprocess.run([binary, "--help"], cwd=cwd, env=env,
                   stdout=subprocess.DEVNULL, check=True)
    cpus = ",".join(map(str, sorted(os.sched_getaffinity(0))[:8]))
    for extra in (["--no-huge-pages"], ["--no-thp"]):
        result = subprocess.run(
            ["taskset", "-c", cpus, str(binary), "--benchmark-seconds", "1",
             "--threads", "1", "--randomx-no-numa", *extra], cwd=cwd, env=env,
            check=True, timeout=180, capture_output=True, text=True,
        )
        output = result.stdout + result.stderr
        print(output, end="", flush=True)
        count = re.search(r"benchmark finished: (\d+) hashes", output)
        if not count or int(count[1]) == 0:
            raise AssertionError("installed miner produced no hashes")
        if "--no-huge-pages" not in extra and "explicit 2 MiB pages:" not in output:
            raise AssertionError("installed miner did not report huge-page coverage")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--git", action="store_true", help="also verify cargo install --git")
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    root = Path(__file__).resolve().parent.parent
    env = os.environ.copy()
    env.pop("RXMINER_ARCH", None)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
        cwd=root, env=env,
    ))
    package = next(p for p in metadata["packages"] if p["name"] == "rxminer")
    name = f"rxminer-{package['version']}"
    with tempfile.TemporaryDirectory(prefix="rxminer-cargo-install-") as temporary:
        temporary = Path(temporary)
        guard = temporary / "blocked-build-tools"
        guard.mkdir()
        guard_log = temporary / "unexpected-build-tool.log"
        for tool in ("cmake", "cmake3", "ccmake", "make", "gmake", "ninja"):
            path = guard / tool
            path.write_text('#!/bin/sh\nprintf "%s\\n" "$0" >> "$RXMINER_BUILD_GUARD_LOG"\nexit 97\n')
            path.chmod(0o755)
        env["PATH"] = str(guard) + os.pathsep + env["PATH"]
        env["RXMINER_BUILD_GUARD_LOG"] = str(guard_log)
        env["CMAKE"] = str(guard / "cmake")
        env["MAKE"] = str(guard / "make")
        run(["cargo", "package", "--locked", "--allow-dirty", "-j", str(args.jobs)],
            cwd=root, env=env)
        install = temporary / "local-installed"
        run(["cargo", "install", "--path", str(root), "--locked", "--bin", "rxminer",
             "--root", str(install), "--target-dir", str(temporary / "local-target"),
             "-j", str(args.jobs)], cwd=temporary, env=env)
        check_binary(install / "bin" / "rxminer", cwd=temporary, env=env)
        if args.git:
            archive = Path(metadata["target_directory"]) / "package" / f"{name}.crate"
            with tarfile.open(archive) as source:
                source.extractall(temporary, filter="data")
            checkout = temporary / name
            (checkout / "Cargo.toml").write_bytes((checkout / "Cargo.toml.orig").read_bytes())
            run(["git", "init", "--quiet", "--initial-branch=main"], cwd=checkout, env=env)
            run(["git", "add", "--all"], cwd=checkout, env=env)
            run(["git", "-c", "user.name=RxMiner install check",
                 "-c", "user.email=install-check@example.invalid",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null",
                 "commit", "--quiet", "-m", "Installation test snapshot"], cwd=checkout, env=env)
            install = temporary / "git-installed"
            run(["cargo", "install", "--git", checkout.as_uri(), "--locked",
                 "--bin", "rxminer", "--root", str(install),
                 "--target-dir", str(temporary / "git-target"), "-j", str(args.jobs)],
                cwd=temporary, env=env)
            check_binary(install / "bin" / "rxminer", cwd=temporary, env=env)
        if guard_log.exists():
            raise AssertionError("installation invoked a blocked build tool: " + guard_log.read_text())
    print("PASS: fresh Cargo installation and hashing without CMake/Make/Ninja", flush=True)


if __name__ == "__main__":
    main()
