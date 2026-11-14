"""Validate the complete release matrix before allowing a draft upload."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile
import zipfile

TARGETS = {
    "linux-x86_64": ("x86_64-unknown-linux-gnu",),
    "linux-aarch64": ("aarch64-unknown-linux-gnu",),
    "linux-riscv64": ("riscv64gc-unknown-linux-gnu", "riscv64a23-unknown-linux-gnu"),
    "macos-x86_64": ("x86_64-apple-darwin",),
    "macos-aarch64": ("aarch64-apple-darwin",),
    "windows-x86_64": ("x86_64-pc-windows-msvc",),
}
BACKENDS = ("rust", "cpp")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_manifest(archive, root, executable):
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as package:
            names = package.namelist()
            manifest = package.read(f"{root}/build-info.json")
    else:
        with tarfile.open(archive, "r:gz") as package:
            entries = package.getmembers()
            require(all(e.isfile() or e.isdir() for e in entries), f"{archive}: unsupported entry")
            names = [e.name for e in entries]
            with package.extractfile(f"{root}/build-info.json") as info:
                manifest = info.read()
    require(len(names) == len(set(names)), f"{archive}: duplicate archive entry")
    for name in names:
        path = PurePosixPath(name.replace("\\", "/"))
        require(not path.is_absolute() and ".." not in path.parts and path.parts[0] == root,
                f"{archive}: unexpected archive path {name}")
    for filename in (executable, "README.md", "LICENSE", "LICENSE-RandomX", "LICENSE-ring",
                     "LICENSE-rustls", "LICENSE-webpki", "Cargo.lock", "config.example.json",
                     "docs/CONFIGURATION.md"):
        require(f"{root}/{filename}" in names, f"{archive}: missing {filename}")
    return json.loads(manifest)


def verify(directory, version, commit, riscv=False):
    expected = set()
    checksums = []
    for platform, targets in TARGETS.items():
        if platform == "linux-riscv64" and not riscv:
            continue
        for backend in BACKENDS:
            root = f"rxminer-v{version}-{platform}-{backend}"
            windows = platform.startswith("windows-")
            name = root + (".zip" if windows else ".tar.gz")
            archive = directory / name
            sidecar = directory / (name + ".sha256")
            expected.update((name, sidecar.name))
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            require(sidecar.read_text().split() == [digest, name], f"{name}: checksum mismatch")
            info = read_manifest(archive, root, "rxminer.exe" if windows else "rxminer")
            for key, value in {"version": version, "commit": commit, "platform": platform,
                               "backend": backend, "features": [f"randomx-{backend}"]}.items():
                require(info.get(key) == value, f"{name}: incorrect {key}")
            require(info.get("target") in targets, f"{name}: incorrect target")
            checksums.append(f"{digest}  {name}\n")
    actual = {p.name for p in directory.iterdir()}
    require(actual == expected, f"unexpected release files: {sorted(actual - expected)}")
    (directory / "SHA256SUMS").write_text("".join(sorted(checksums)))
    print(f"Verified {len(checksums)} archives: exact matrix, checksums, contents and build identity")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--riscv", action="store_true")
    args = parser.parse_args()
    verify(args.directory, args.version, args.commit, args.riscv)
