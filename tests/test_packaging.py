"""Checks for release layout and publish-time path/overwrite safety."""

import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
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
        (self.root / "skills" / "cpyref" / "agents").mkdir(parents=True)
        (self.root / "LICENSE").write_bytes(b"AUL license\n")
        (self.root / "LICENSE.MIT").write_bytes(b"Historical MIT license\n")
        (self.root / "AGENTS.md").write_bytes(b"Agent instructions\n")
        (self.root / "UPSTREAM.toml").write_bytes(b"schema_version = 1\n")
        (self.root / "README.md").write_bytes(b"# CpyRef\n")
        (self.root / "docs" / "nested" / "usage.md").write_bytes(b"usage\n")
        (self.root / "skills" / "cpyref" / "SKILL.md").write_bytes(b"---\nname: cpyref\n---\n")
        (self.root / "skills" / "cpyref" / "agents" / "openai.yaml").write_bytes(b"interface:\n  display_name: CpyRef\n")
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
                    self.assertEqual(archive.extractfile(base + "/LICENSE").read(), b"AUL license\n")
                    self.assertEqual(archive.extractfile(base + "/LICENSE.MIT").read(), b"Historical MIT license\n")
                    self.assertEqual(archive.extractfile(base + "/AGENTS.md").read(), b"Agent instructions\n")
                    self.assertEqual(archive.extractfile(base + "/UPSTREAM.toml").read(), b"schema_version = 1\n")
                    self.assertEqual(archive.extractfile(base + "/README.md").read(), b"# CpyRef\n")
                    self.assertEqual(archive.extractfile(base + "/docs/nested/usage.md").read(), b"usage\n")
                    self.assertEqual(archive.getmember(base + "/cpyref").mode, 0o755)
                    self.assertEqual(archive.getmember(base + "/docs/nested").mode, 0o755)
                    self.assertEqual(archive.getmember(base + "/LICENSE").mode, 0o644)
                    self.assertEqual(archive.extractfile(base + "/skills/cpyref/SKILL.md").read(), b"---\nname: cpyref\n---\n")
                    self.assertEqual(archive.extractfile(base + "/skills/cpyref/agents/openai.yaml").read(), b"interface:\n  display_name: CpyRef\n")
                    self.assertEqual(archive.getmember(base + "/skills/cpyref/agents").mode, 0o755)
                    self.assertEqual(archive.getmember(base + "/skills/cpyref/agents/openai.yaml").mode, 0o644)
                    self.assertTrue(all(member.uid == member.gid == 0 for member in archive.getmembers()))

    def test_windows_zip_preserves_layout_bytes_and_unix_modes(self):
        target = "aarch64-pc-windows-msvc"
        result = package.create_archive(self.root, self.binary, target, "1.2.3", self.output)
        base = f"cpyref-1.2.3-{target}"
        with zipfile.ZipFile(result) as archive:
            self.assertEqual(archive.read(base + "/cpyref.exe"), b"binary\x00\xff\n")
            self.assertEqual(archive.read(base + "/LICENSE"), b"AUL license\n")
            self.assertEqual(archive.read(base + "/LICENSE.MIT"), b"Historical MIT license\n")
            self.assertEqual(archive.read(base + "/AGENTS.md"), b"Agent instructions\n")
            self.assertEqual(archive.read(base + "/UPSTREAM.toml"), b"schema_version = 1\n")
            self.assertEqual(archive.read(base + "/docs/nested/usage.md"), b"usage\n")
            self.assertEqual(archive.read(base + "/skills/cpyref/SKILL.md"), b"---\nname: cpyref\n---\n")
            self.assertEqual(archive.read(base + "/skills/cpyref/agents/openai.yaml"), b"interface:\n  display_name: CpyRef\n")
            self.assertTrue(archive.getinfo(base + "/docs/nested/").is_dir())
            self.assertEqual(stat.S_IMODE(archive.getinfo(base + "/cpyref.exe").external_attr >> 16), 0o755)
            self.assertEqual(stat.S_IMODE(archive.getinfo(base + "/README.md").external_attr >> 16), 0o644)
            self.assertEqual(stat.S_IMODE(archive.getinfo(base + "/skills/cpyref/agents/").external_attr >> 16), 0o755)
            self.assertEqual(stat.S_IMODE(archive.getinfo(base + "/skills/cpyref/agents/openai.yaml").external_attr >> 16), 0o644)
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

    @unittest.skipIf(os.name == "nt", "creating symlinks on Windows requires privileges")
    def test_skill_tree_and_ancestor_symlinks_are_rejected(self):
        outside = self.root / "outside"
        outside.mkdir()
        for link in (self.root / "skills" / "cpyref" / "linked", self.root / "skills" / "cpyref" / "agents" / "linked"):
            with self.subTest(link=link):
                link.symlink_to(outside, target_is_directory=True)
                try:
                    with self.assertRaisesRegex(ValueError, "symlinks"):
                        package.create_archive(self.root, self.binary, package.TARGETS[0], "1.2.3", self.output)
                finally:
                    link.unlink()
        (self.root / "skills").rename(self.root / "real-skills")
        (self.root / "skills").symlink_to(self.root / "real-skills", target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink"):
            package.create_archive(self.root, self.binary, package.TARGETS[0], "1.2.3", self.output)
        self.assertFalse(self.output.exists())

    def test_incomplete_skill_is_rejected_before_output_creation(self):
        (self.root / "skills" / "cpyref" / "agents" / "openai.yaml").unlink()
        with self.assertRaisesRegex(ValueError, "regular file"):
            package.create_archive(self.root, self.binary, package.TARGETS[0], "1.2.3", self.output)
        self.assertFalse(self.output.exists())

    @unittest.skipUnless(shutil.which("cargo"), "Cargo required for source-package privacy check")
    def test_cargo_package_excludes_private_files_with_public_resource_names(self):
        # Exercise Cargo's actual glob rules, not a Python approximation of them.
        match = re.search(r"^include\s*=\s*(\[.*\])$", (REPO / "Cargo.toml").read_text(), re.MULTILINE)
        includes = json.loads(match.group(1))
        (self.root / "src").mkdir()
        (self.root / "src" / "lib.rs").write_text("")
        (self.root / "Cargo.toml").write_text(
            '[package]\nname="cpyref-package-privacy-check"\nversion="0.0.0"\nedition="2021"\n'
            'license="MIT"\ninclude=' + json.dumps(includes) + '\n[workspace]\n')
        forbidden = ["benchmarks/private/README.md", "private/AGENTS.md", "private/LICENSE",
                     "private/UPSTREAM.toml", "stored/c0123456789ab.ref", "target/private.txt"]
        for name in forbidden:
            p = self.root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("SYNTHETIC PRIVATE DATA")
        result = subprocess.run(["cargo", "package", "--list", "--offline", "--allow-dirty"],
                                cwd=self.root, capture_output=True, text=True, timeout=30,
                                env={**os.environ, "CARGO_HOME": str(REPO / "target" / "cargo-home")})
        self.assertEqual(result.returncode, 0, result.stderr)
        names = {name.replace("\\", "/") for name in result.stdout.splitlines()}
        self.assertIn("README.md", names)
        self.assertIn("docs/nested/usage.md", names)
        self.assertIn("skills/cpyref/SKILL.md", names)
        self.assertTrue(names.isdisjoint(forbidden), names.intersection(forbidden))


if __name__ == "__main__":
    unittest.main()
