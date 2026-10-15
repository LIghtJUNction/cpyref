"""Self-tests for evidence accounting, NOT experimental observations."""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from tasks import make_tasks

ROOT = Path(__file__).resolve().parent

class AccountingTests(unittest.TestCase):
    def build(self, root):
        env = {key: 'same' for key in ['model_sha256','model_revision','llama_tag','llama_archive_sha256','docker_image']}
        rows = []
        for task in make_tasks():
            for arm in ['native','cpyref']:
                row = dict(task=task['id'],arm=arm,passed=task['id']=='extract_table',
                    prompt_tokens=100,completion_tokens=10,model_calls=1,tool_calls=1,
                    cpyref_command_calls=int(arm=='cpyref'),elapsed_seconds=2,
                    acceptance_seconds=0.5,inference_error=None,usage_complete=True)
                rows.append(row)
                folder=root/'shard-0'/f'{task["id"]}-{arm}'
                folder.mkdir(parents=True)
                (folder/'trace.json').write_text(json.dumps([{'response':{'usage':{'prompt_tokens':100,'completion_tokens':10}}}]))
        (root/'shard-0'/'results.json').write_text(json.dumps(rows))
        (root/'shard-0'/'environment.json').write_text(json.dumps(env))
        return rows

    def run_report(self, root):
        return subprocess.run([sys.executable,str(ROOT/'report.py'),str(root)],capture_output=True,text=True)

    def test_failed_attempts_are_charged(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); self.build(root)
            out=self.run_report(root)
            self.assertEqual(out.returncode,0,out.stderr)
            summary=json.loads((root/'summary.json').read_text())
            native=summary['totals']['native']
            self.assertEqual(native['successes'],1)
            self.assertEqual(native['total_tokens'],660)
            self.assertEqual(native['tokens_per_success'],660)
            self.assertEqual(native['seconds_per_success'],15)
            self.assertIsNone(summary['compute_cost_usd'])

    def test_missing_episode_is_not_a_zero_cost_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); rows=self.build(root)
            (root/'shard-0'/'results.json').write_text(json.dumps(rows[:-1]))
            self.assertNotEqual(self.run_report(root).returncode,0)
            summary=json.loads((root/'summary.json').read_text())
            self.assertFalse(summary['audit']['valid_complete_pilot'])

    def test_usage_mismatch_fails_audit(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); rows=self.build(root); rows[0]['prompt_tokens']=99
            (root/'shard-0'/'results.json').write_text(json.dumps(rows))
            self.assertNotEqual(self.run_report(root).returncode,0)
            summary=json.loads((root/'summary.json').read_text())
            self.assertTrue(any('mismatch' in e for e in summary['audit']['errors']))

    def test_no_success_has_undefined_unit_cost(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); rows=self.build(root)
            for row in rows: row['passed']=False
            (root/'shard-0'/'results.json').write_text(json.dumps(rows))
            self.assertEqual(self.run_report(root).returncode,0)
            summary=json.loads((root/'summary.json').read_text())
            self.assertIsNone(summary['totals']['native']['tokens_per_success'])
            self.assertIsNone(summary['totals']['cpyref']['price_ratio_sensitivity']['4']['units_per_success'])

if __name__=='__main__':
    unittest.main()
