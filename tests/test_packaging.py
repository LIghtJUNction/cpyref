"""Checks for release layout and publish-time path/overwrite safety."""

import importlib.util
import os
from pathlib import Path
import stat
import tarfile
import tempfile
import unittest
from unittest import mock
import zipfile


REPO = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("package", REPO / "scripts" / "package.py")
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix=".packaging-test-", dir=REPO)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "docs" / "nested").mkdir(parents=True)
        (self.root / "LICENSE").write_bytes(b"MIT license\n")
        (self.root / "README.md").write_bytes(b"# CpyRef\n")
        (self.root / "docs" / "nested" / "usage.md").write_bytes(b"usage\n")
        self.binary = self.root / "input-binary"
        self.binary.write_bytes(b"binary\x00\xff\n")
        self.output = self.root / "dist"
        self.environment = mock.patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "1000000000"})
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def test_unix_archive_layout_content_and_permissions(self):
        for target in ("x86_64-unknown-linux-musl", "aarch64-apple-darwin"):
            with self.subTest(target=target):
                result = package.create_archive(self.root, self.binary, target, "1.2.3", self.output)
                base = f"cpyref-1.2.3-{target}"
                with tarfile.open(result) as archive:
                    self.assertTrue(all(name == base or name.startswith(base + "/") for name in archive.getnames()))
                    self.assertEqual(archive.extractfile(base + "/cpyref").read(), b"binary\x00\xff\n")
                    self.assertEqual(archive.extractfile(base + "/LICENSE").read(), b"MIT license\n")
                    self.assertEqual(archive.extractfile(base + "/README.md").read(), b"# CpyRef\n")
                    self.assertEqual(archive.extractfile(base + "/docs/nested/usage.md").read(), b"usage\n")
                    self.assertEqual(archive.getmember(base + "/cpyref").mode, 0o755)
                    self.assertEqual(archive.getmember(base + "/docs/nested").mode, 0o755)
                    self.assertEqual(archive.getmember(base + "/LICENSE").mode, 0o644)
                    self.assertTrue(all(member.uid == member.gid == 0 for member in archive.getmembers()))

    def test_windows_zip_preserves_layout_bytes_and_unix_modes(self):
        target = "aarch64-pc-windows-msvc"
        result = package.create_archive(self.root, self.binary, target, "1.2.3", self.output)
        base = f"cpyref-1.2.3-{target}"
        with zipfile.ZipFile(result) as archive:
            self.assertEqual(archive.read(base + "/cpyref.exe"), b"binary\x00\xff\n")
            self.assertEqual(archive.read(base + "/docs/nested/usage.md"), b"usage\n")
            self.assertTrue(archive.getinfo(base + "/docs/nested/").is_dir())
            self.assertEqual(stat.S_IMODE(archive.getinfo(base + "/cpyref.exe").external_attr >> 16), 0o755)
            self.assertEqual(stat.S_IMODE(archive.getinfo(base + "/README.md").external_attr >> 16), 0o644)
            self.assertTrue(all(name.startswith(base + "/") for name in archive.namelist()))

    def test_invalid_versions_and_targets_cannot_create_outputs(self):
        for version in ("v1.2.3", "01.2.3", "1.2", "1.2.3-01", "1.2.3/../../x", "1.2.3\n", "$(id)"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                package.create_archive(self.root, self.binary, package.TARGETS[0], version, self.output)
        with self.assertRaises(ValueError):
            package.create_archive(self.root, self.binary, "../../other", "1.2.3", self.output)
        self.assertFalse(self.output.exists())

    def test_missing_binary_is_rejected_before_output_creation(self):
        with self.assertRaisesRegex(ValueError, "regular file"):
            package.create_archive(self.root, self.root / "missing", package.TARGETS[0], "1.2.3", self.output)
        self.assertFalse(self.output.exists())

    def test_existing_archive_is_never_overwritten(self):
        result = package.create_archive(self.root, self.binary, package.TARGETS[0], "1.2.3", self.output)
        original = result.read_bytes()
        self.binary.write_bytes(b"different")
        with self.assertRaises(FileExistsError):
            package.create_archive(self.root, self.binary, package.TARGETS[0], "1.2.3", self.output)
        self.assertEqual(result.read_bytes(), original)
        self.assertEqual(list(self.output.iterdir()), [result])

    @unittest.skipIf(os.name == "nt", "creating symlinks on Windows requires privileges")
    def test_symlink_in_docs_cannot_leak_external_content(self):
        (self.root / "docs" / "link").symlink_to(self.binary)
        with self.assertRaisesRegex(ValueError, "symlinks"):
            package.create_archive(self.root, self.binary, package.TARGETS[0], "1.2.3", self.output)
        self.assertFalse(self.output.exists())

    def test_archive_is_reproducible_when_input_mtimes_change(self):
        first = package.create_archive(self.root, self.binary, package.TARGETS[0], "1.2.3", self.output)
        os.utime(self.binary, (2000000000, 2000000000))
        second = package.create_archive(self.root, self.binary, package.TARGETS[0], "1.2.3", self.root / "another-dist")
        self.assertEqual(first.read_bytes(), second.read_bytes())


if __name__ == "__main__":
    unittest.main()
