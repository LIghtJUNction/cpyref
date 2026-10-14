"""Cross-platform CLI regression tests. All files and secrets below are synthetic."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("CPYREF_BIN", ROOT / "target" / "debug" / ("cpyref.exe" if os.name == "nt" else "cpyref"))).resolve()


@unittest.skipUnless(BINARY.is_file(), "Build cpyref first, or set CPYREF_BIN")
class HardeningTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cpyref-check-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / ".git").mkdir()
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("CPYREF_")}
        self.env.update(CPYREF_NO_CONFIG="true", CPYREF_STORE=str(self.root / "store"))
        self.source = self.root / "source"
        self.target = self.root / "target"
        self.source.write_bytes(b"new\x00text\r\n")
        self.target.write_bytes(b"first\r\nold\r\nlast\r\n")

    def call(self, *args, timeout=15):
        return subprocess.run([str(BINARY), *map(str, args)], cwd=self.root, env=self.env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout, check=False)

    def good(self, *args):
        result = self.call(*args)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        return json.loads(result.stdout)

    def copy(self, namespace=None):
        args = ["--namespace", namespace] if namespace else []
        result = self.call(*args, "copy", self.source)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.decode().strip()

    def reject(self, code, *args):
        before = self.target.read_bytes()
        result = self.call(*args)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(json.loads(result.stderr)["error"]["code"], code)
        self.assertEqual(self.target.read_bytes(), before)

    def test_insert_preserves_binary_and_crlf_and_does_not_expand_payload(self):
        ref = self.copy()
        preview = self.good("insert", ref, self.target, "--before-line", 2, "--dry-run")
        self.assertFalse(preview["applied"])
        self.assertEqual(preview["before_sha256"], hashlib.sha256(self.target.read_bytes()).hexdigest())
        self.assertNotIn("new", json.dumps(preview))
        report = self.good("insert", ref, self.target, "--before-line", 2,
                           "--expect-sha256", preview["before_sha256"])
        self.assertTrue(report["applied"])
        self.assertEqual(self.target.read_bytes(), b"first\r\nnew\x00text\r\nold\r\nlast\r\n")
        self.assertEqual(hashlib.sha256(self.target.read_bytes()).hexdigest(), preview["after_sha256"])

    def test_replace_unique_text_and_inclusive_lines(self):
        ref = self.copy()
        for selector, value, expected in [
            ("--text", "old", b"first\r\nnew\x00text\r\n\r\nlast\r\n"),
            ("--lines", "2:2", b"first\r\nnew\x00text\r\nlast\r\n"),
        ]:
            with self.subTest(selector=selector):
                self.target.write_bytes(b"first\r\nold\r\nlast\r\n")
                preview = self.good("replace", ref, self.target, selector, value, "--dry-run")
                self.good("replace", ref, self.target, selector, value, "--expect-sha256", preview["before_sha256"])
                self.assertEqual(self.target.read_bytes(), expected)

    def test_stale_hash_and_repeat_are_rejected_without_writes(self):
        ref = self.copy()
        preview = self.good("insert", ref, self.target, "--at-end", "--dry-run")
        self.target.write_bytes(b"other agent update\n")
        self.reject("TARGET_CHANGED", "insert", ref, self.target, "--at-end", "--expect-sha256", preview["before_sha256"])
        current = self.good("insert", ref, self.target, "--at-end", "--dry-run")
        self.good("insert", ref, self.target, "--at-end", "--expect-sha256", current["before_sha256"])
        self.reject("TARGET_CHANGED", "insert", ref, self.target, "--at-end", "--expect-sha256", current["before_sha256"])

    def test_ambiguous_missing_and_invalid_positions_are_read_only(self):
        ref = self.copy()
        self.target.write_bytes(b"same same")
        self.reject("AMBIGUOUS", "replace", ref, self.target, "--text", "same", "--dry-run")
        self.reject("NO_MATCH", "replace", ref, self.target, "--text", "absent", "--dry-run")
        self.reject("INVALID_ARGUMENT", "insert", ref, self.target, "--before-line", 0, "--dry-run")
        self.reject("INVALID_ARGUMENT", "insert", ref, self.target, "--at-start", "--at-end", "--dry-run")
        self.reject("PRECONDITION_REQUIRED", "insert", ref, self.target, "--at-start")

    def test_empty_target_and_text_anchor(self):
        ref = self.copy()
        self.target.write_bytes(b"")
        preview = self.good("insert", ref, self.target, "--at-start", "--dry-run")
        self.good("insert", ref, self.target, "--at-start", "--expect-sha256", preview["before_sha256"])
        self.assertEqual(self.target.read_bytes(), self.source.read_bytes())
        self.target.write_bytes(b"<anchor>")
        preview = self.good("insert", ref, self.target, "--after-text", "anchor", "--dry-run")
        self.good("insert", ref, self.target, "--after-text", "anchor", "--expect-sha256", preview["before_sha256"])
        self.assertEqual(self.target.read_bytes(), b"<anchor" + self.source.read_bytes() + b">")

    @unittest.skipIf(os.name == "nt", "Unix mode and link semantics")
    def test_permissions_and_unsafe_links(self):
        ref = self.copy()
        self.target.chmod(0o751)
        preview = self.good("insert", ref, self.target, "--at-end", "--dry-run")
        self.good("insert", ref, self.target, "--at-end", "--expect-sha256", preview["before_sha256"])
        self.assertEqual(self.target.stat().st_mode & 0o777, 0o751)
        link = self.root / "link"
        link.symlink_to(self.target)
        result = self.call("insert", ref, link, "--at-end", "--dry-run")
        self.assertEqual(json.loads(result.stderr)["error"]["code"], "UNSAFE_OUTPUT")
        self.target.chmod(0o444)
        self.addCleanup(lambda: self.target.chmod(0o600) if self.target.exists() else None)
        self.reject("PERMISSION_DENIED", "insert", ref, self.target, "--at-end", "--dry-run")

    def test_two_namespaces_cannot_overwrite_each_others_edit(self):
        refs = [self.copy(namespace) for namespace in ("a", "b")]
        expected = hashlib.sha256(self.target.read_bytes()).hexdigest()
        processes = [subprocess.Popen([str(BINARY), "--namespace", ns, "insert", ref, str(self.target),
                                      "--at-end", "--expect-sha256", expected],
                                     cwd=self.root, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                     for ns, ref in zip(("a", "b"), refs)]
        results = [process.communicate(timeout=15) for process in processes]
        self.assertEqual(sorted(process.returncode for process in processes), [0, 10], results)
        self.assertEqual(self.target.read_bytes(), b"first\r\nold\r\nlast\r\n" + self.source.read_bytes())

    def test_normal_exit_and_exact_log_budget(self):
        result = self.call("run", "--max-log-bytes", 128, "--", sys.executable,
                           "-c", "import sys; sys.stdout.buffer.write(b'x'*128)")
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertIsNone(report["termination_reason"])
        self.assertTrue(report["logs_complete"])
        self.assertEqual(report["stdout_bytes"], 128)
        result = self.call("run", "--", sys.executable, "-c", "raise SystemExit(7)")
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertEqual(json.loads(result.stdout)["command_exit"], 7)

    def test_timeout_is_not_reported_as_test_success(self):
        start = time.monotonic()
        result = self.call("run", "--timeout", 1, "--", sys.executable,
                           "-c", "import time; print('started', flush=True); time.sleep(10)")
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertLess(time.monotonic() - start, 8)
        report = json.loads(result.stdout)
        self.assertEqual(report["termination_reason"], "timeout")
        self.assertFalse(report["logs_complete"])
        self.assertIsNone(report["analysis"]["counts"])
        saved = self.good("logs", report["reference"])
        self.assertEqual(saved["termination_reason"], "timeout")

    def test_descendant_holding_pipes_is_terminated(self):
        child = "import time,pathlib; time.sleep(3); pathlib.Path('descendant-survived').write_text('bad')"
        parent = "import subprocess,sys; subprocess.Popen([sys.executable, '-c', " + repr(child) + "])"
        result = self.call("run", "--timeout", 1, "--", sys.executable, "-c", parent)
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertEqual(json.loads(result.stdout)["termination_reason"], "timeout")
        time.sleep(2.3)
        self.assertFalse((self.root / "descendant-survived").exists())

    def test_log_budget_is_shared_by_both_streams(self):
        program = "import sys; sys.stdout.buffer.write(b'a'*100000); sys.stdout.flush(); sys.stderr.buffer.write(b'b'*100000)"
        result = self.call("run", "--max-log-bytes", 1024, "--", sys.executable, "-c", program)
        self.assertEqual(result.returncode, 125, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["termination_reason"], "log_limit")
        self.assertFalse(report["logs_complete"])
        self.assertLessEqual(report["stdout_bytes"] + report["stderr_bytes"], 1024)
        self.assertIsNone(report["analysis"]["counts"])

    def test_declared_secret_is_not_stored_in_logs_or_argv(self):
        secret = "synthetic-credential-abcdefgh"
        self.env["DEMO_SECRET"] = secret
        program = "import os,sys,time; s=os.environ['DEMO_SECRET']; sys.stdout.write(s[:10]); sys.stdout.flush(); time.sleep(.05); sys.stdout.write(s[10:]); sys.stderr.write(s)"
        result = self.call("run", "--redact-env", "DEMO_SECRET", "--", sys.executable, "-c", program, secret)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["redacted_env"], ["DEMO_SECRET"])
        for record in (self.root / "store").rglob("*.ref"):
            self.assertNotIn(secret.encode(), record.read_bytes())
        for stream in ("stdout", "stderr"):
            log = self.good("logs", report["reference"], "--stream", stream)
            self.assertEqual(log["content"], "[REDACTED]")
        args = self.good("logs", report["reference"], "--view", "command")
        self.assertNotIn(secret, json.dumps(args))

    def test_invalid_redaction_fails_before_starting(self):
        self.env.pop("MISSING_DEMO_SECRET", None)
        result = self.call("run", "--redact-env", "MISSING_DEMO_SECRET", "--", sys.executable,
                           "-c", "from pathlib import Path; Path('started').touch()")
        self.assertEqual(result.returncode, 4)
        self.assertEqual(json.loads(result.stderr)["error"]["code"], "REDACTION_CONFIG")
        self.assertFalse((self.root / "started").exists())


if __name__ == "__main__":
    unittest.main()
