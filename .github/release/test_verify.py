import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

from verify import BACKENDS, TARGETS, verify


class ReleaseMatrixTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)

    def archive(self, platform, artifact_backend, **overrides):
        backend = artifact_backend
        root = f"rxminer-v1.2.3-{platform}-{backend}"
        windows = platform.startswith("windows-")
        path = self.directory / (root + (".zip" if windows else ".tar.gz"))
        info = dict(version="1.2.3", commit="test-commit", platform=platform,
                    backend=backend, features=[f"randomx-{backend}"], target=TARGETS[platform][0])
        info.update(overrides)
        files = {name: b"fixture" for name in (
            "rxminer.exe" if windows else "rxminer", "README.md", "LICENSE", "LICENSE-RandomX",
            "LICENSE-ring", "LICENSE-rustls", "LICENSE-webpki", "Cargo.lock", "config.example.json",
            "docs/CONFIGURATION.md")}
        files["build-info.json"] = json.dumps(info).encode()
        if windows:
            with zipfile.ZipFile(path, "w") as archive:
                for name, data in files.items():
                    archive.writestr(f"{root}/{name}", data)
        else:
            with tarfile.open(path, "w:gz") as archive:
                for name, data in files.items():
                    entry = tarfile.TarInfo(f"{root}/{name}")
                    entry.size = len(data)
                    archive.addfile(entry, io.BytesIO(data))
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        path.with_name(path.name + ".sha256").write_text(f"{digest}  {path.name}\n")
        return path

    def matrix(self, riscv=False):
        for platform in TARGETS:
            if platform != "linux-riscv64" or riscv:
                for backend in BACKENDS:
                    self.archive(platform, backend)

    def test_ten_and_twelve_archive_sets(self):
        self.matrix()
        verify(self.directory, "1.2.3", "test-commit")
        sums = self.directory / "SHA256SUMS"
        self.assertEqual(len(sums.read_text().splitlines()), 10)
        sums.unlink()
        self.matrix(riscv=True)
        verify(self.directory, "1.2.3", "test-commit", riscv=True)
        self.assertEqual(len(sums.read_text().splitlines()), 12)

    def test_missing_variant_fails(self):
        self.matrix()
        (self.directory / "rxminer-v1.2.3-windows-x86_64-cpp.zip").unlink()
        with self.assertRaises(FileNotFoundError):
            verify(self.directory, "1.2.3", "test-commit")
        self.assertFalse((self.directory / "SHA256SUMS").exists())

    def test_corrupt_checksum_fails(self):
        self.matrix()
        path = self.directory / "rxminer-v1.2.3-linux-x86_64-rust.tar.gz"
        path.write_bytes(path.read_bytes() + b"changed")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            verify(self.directory, "1.2.3", "test-commit")

    def test_relabelled_or_test_build_fails(self):
        self.matrix()
        for override in (dict(backend="rust"), dict(features=["randomx-cpp", "test-dev-fee"]),
                         dict(commit="other-commit"), dict(target="aarch64-unknown-linux-gnu")):
            with self.subTest(override=override):
                self.archive("windows-x86_64", "cpp", **override)
                with self.assertRaises(ValueError):
                    verify(self.directory, "1.2.3", "test-commit")

    def test_stale_extra_variant_fails(self):
        self.matrix(riscv=True)
        with self.assertRaisesRegex(ValueError, "unexpected release files"):
            verify(self.directory, "1.2.3", "test-commit")


if __name__ == "__main__":
    unittest.main()
