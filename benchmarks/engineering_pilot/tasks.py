"""Constructed maintenance fixtures, not mined real-world issues.
Golden outputs and acceptance checks stay outside the agent workspace.
"""
from __future__ import annotations
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path

SEED = 17

def token(label: str) -> str:
    return hashlib.sha256(f"{SEED}:{label}".encode()).hexdigest()[:24]

def make_tasks() -> list[dict]:
    table = "TABLE = {\n" + "".join(f'    "code_{i:03}": "{token(str(i))}",\n' for i in range(96)) + "}\n"
    catalog = '"""Public lookup API."""\n\n# INSERT_TABLE\n\ndef lookup(key):\n    return TABLE[key]\n'
    parser = '''def parse_duration(text):
    """Convert a duration with ms, s, m, or h units to milliseconds.

    The input must be text. Reject negative, non-finite, or unitless values.
    Fractional values are allowed only when the result is whole milliseconds.
    Preserve these exception types for callers of the compatibility API.
    """
    import re
    from decimal import Decimal

    if not isinstance(text, str):
        raise TypeError("duration must be text")
    match = re.fullmatch(r"\\s*(\\d+(?:\\.\\d+)?)\\s*(ms|s|m|h)\\s*", text)
    if match is None:
        raise ValueError("invalid duration")
    units = {"ms": 1, "s": 1000, "m": 60000, "h": 3600000}
    milliseconds = Decimal(match.group(1)) * units[match.group(2)]
    if milliseconds != milliseconds.to_integral_value():
        raise ValueError("fractional milliseconds are not supported")
    return int(milliseconds)
'''
    compat = '"""Compatibility entry points."""\n\n# INSERT_PARSER\n\nAPI_VERSION = 1\n'
    tuning = '[service_tuning]\n' + ''.join(f'route_{i:02} = {token("route" + str(i))}\n' for i in range(48)) + '\n'
    dev = '[server]\nhost = localhost\nport = 8080\n\n# INSERT_TUNING\n[logging]\nlevel = DEBUG\n'
    prod = '[server]\nhost = 0.0.0.0\nport = 443\n\n# INSERT_TUNING\n[logging]\nlevel = WARNING\n'
    fixture = json.dumps({'items': [{'id': i, 'label': token('fixture'+str(i)), 'active': i % 3 == 0} for i in range(80)]}, indent=2) + '\n'
    retry = '''def backoff(attempt):
    if not isinstance(attempt, int) or attempt < 0:
        raise ValueError("attempt must be a nonnegative integer")
    return min(60, 2 ** (attempt + 1))
'''
    retry_tests = '''import unittest
from retry import backoff

class RetryTests(unittest.TestCase):
    def test_backoff(self):
        for i in range(600):
            print("trace: processed request %04d; status=ok" % i)
        self.assertEqual([backoff(i) for i in range(8)], [1, 2, 4, 8, 16, 32, 60, 60])

if __name__ == "__main__":
    unittest.main()
'''
    labels = 'def unique_labels(items):\n    return sorted(set(items))\n'
    fixed_labels = '''def unique_labels(items):
    result, seen = [], set()
    for item in items:
        if not isinstance(item, str):
            raise TypeError("labels must be strings")
        clean = item.strip()
        key = clean.casefold()
        if clean and key not in seen:
            seen.add(key)
            result.append(clean)
    return result
'''
    fixture_test = '''import json
from pathlib import Path
for name in ["fixtures/users.json", "fixtures/orders.json", "fixtures/events.json"]:
    value = json.loads(Path(name).read_text())
    assert len(value["items"]) == 80
print("fixture contract passed")
'''
    tasks = [
        dict(id='extract_table', category='partial_reuse',
             instruction='Create the shared catalog TABLE in catalog.py by inserting the exact bytes between # TABLE_START and # TABLE_END in legacy.py immediately after the # INSERT_TABLE line. Do not copy the marker lines. Keep legacy.py and all other catalog.py bytes unchanged. The lookup function must work.',
             files={'legacy.py': '# Legacy API retained.\n# TABLE_START\n'+table+'# TABLE_END\n\ndef legacy_lookup(k):\n    return TABLE[k]\n', 'catalog.py': catalog},
             golden={'catalog.py': catalog.replace('# INSERT_TABLE\n', '# INSERT_TABLE\n'+table)},
             check='import catalog\nassert catalog.lookup("code_017") == '+repr(token('17'))+'\n'),
        dict(id='port_parser', category='function_reuse',
             instruction='Expose the existing parse_duration function from parser_source.py in compat.py. Insert the whole function, unchanged byte-for-byte, after the # INSERT_PARSER line. Do not copy the marker comments or the helper outside that block. Preserve parser_source.py and the existing contents of compat.py.',
             files={'parser_source.py': '# PARSER_START\n'+parser+'# PARSER_END\n\ndef unrelated():\n    return "do not copy"\n', 'compat.py': compat},
             golden={'compat.py': compat.replace('# INSERT_PARSER\n', '# INSERT_PARSER\n'+parser)},
             check='from compat import parse_duration as p\nassert [p(x) for x in ["1ms", " 1.5s ", "2m", "0.25h"]] == [1,1500,120000,900000]\nfor x in ["-1s", "1", "nan s", "0.1ms"]:\n    try: p(x)\n    except ValueError: pass\n    else: raise AssertionError(x)\n'),
        dict(id='sync_config', category='multi_target_reuse',
             instruction='Add the exact service tuning block from tuning_source.ini (between # TUNING_START and # TUNING_END, excluding the markers) to dev.ini and prod.ini immediately after each # INSERT_TUNING line. Preserve all existing target bytes and the source file. Both resulting files must remain valid INI.',
             files={'tuning_source.ini': '# TUNING_START\n'+tuning+'# TUNING_END\n', 'dev.ini': dev, 'prod.ini': prod},
             golden={'dev.ini': dev.replace('# INSERT_TUNING\n', '# INSERT_TUNING\n'+tuning), 'prod.ini': prod.replace('# INSERT_TUNING\n', '# INSERT_TUNING\n'+tuning)},
             check='import configparser\nfor name in ["dev.ini", "prod.ini"]:\n    c=configparser.ConfigParser(); c.read(name)\n    assert len(c["service_tuning"]) == 48\n'),
        dict(id='fixture_copy', category='whole_file_control',
             instruction='Backfill the three JSON fixtures required by test_fixtures.py from canonical.json. Their bytes must exactly match the canonical file. Do not modify canonical.json or the test. Run the existing test to check the result.',
             files={'canonical.json': fixture, 'test_fixtures.py': fixture_test},
             golden={name: fixture for name in ['fixtures/users.json', 'fixtures/orders.json', 'fixtures/events.json']},
             check='exec(open("test_fixtures.py").read())\n'),
        dict(id='noisy_retry_bug', category='noisy_test_diagnosis',
             instruction='Fix the backoff calculation in retry.py so the existing test_retry.py passes. The test emits verbose diagnostics. Keep rejecting negative or non-integer attempts with ValueError. Preserve test_retry.py. Do not replace tests with easier assertions.',
             files={'retry.py': retry, 'test_retry.py': retry_tests},
             golden={'retry.py': retry.replace('(attempt + 1)', 'attempt')}, exact=False,
             check='from retry import backoff\nassert [backoff(i) for i in range(9)] == [1,2,4,8,16,32,60,60,60]\nfor x in [-1,1.5,"1"]:\n    try: backoff(x)\n    except ValueError: pass\n    else: raise AssertionError(x)\n'),
        dict(id='labels_logic', category='no_copy_control',
             instruction='Implement unique_labels(items) in labels.py. Strip leading/trailing whitespace; discard empty strings; deduplicate using casefold; preserve input order and the first cleaned spelling. Raise TypeError for any non-string item. Do not sort. Example: ["  Alpha ", "alpha", "", " BETA "] becomes ["Alpha", "BETA"].',
             files={'labels.py': labels}, golden={'labels.py': fixed_labels}, exact=False,
             check='from labels import unique_labels as f\nassert f(["  Alpha ","alpha",""," BETA "]) == ["Alpha","BETA"]\nassert f(["Straße", "STRASSE", "z", "Z"]) == ["Straße","z"]\nassert f([]) == []\ntry: f(["ok", 3])\nexcept TypeError: pass\nelse: raise AssertionError("must reject non-string")\n'),
    ]
    for task in tasks:
        task['exact'] = task.get('exact', True)
        protected = {p: text for p, text in task['files'].items() if p not in task['golden']}
        checks = ['from pathlib import Path\n']
        for name, text in protected.items():
            checks.append(f'assert Path({name!r}).read_bytes() == {text.encode()!r}, "protected file modified: {name}"\n')
        if task['exact']:
            for name, text in task['golden'].items():
                checks.append(f'assert Path({name!r}).read_bytes() == {text.encode()!r}, "wrong bytes: {name}"\n')
        checks.append(task['check'])
        task['oracle'] = ''.join(checks)
    return tasks

def materialize(task: dict, root: Path, golden: bool = False) -> None:
    root.mkdir(parents=True, exist_ok=True)
    (root / '.git').mkdir(exist_ok=True)
    for name, text in (task['files'] | (task['golden'] if golden else {})).items():
        p = root / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(text.encode())

def selftest() -> None:
    for task in make_tasks():
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            materialize(task, root)
            before = subprocess.run([sys.executable, '-B', '-c', task['oracle']], cwd=root, capture_output=True)
            assert before.returncode != 0, (task['id'], 'fixture starts solved')
            materialize(task, root, golden=True)
            after = subprocess.run([sys.executable, '-B', '-c', task['oracle']], cwd=root, capture_output=True)
            assert after.returncode == 0, (task['id'], after.stderr.decode())
        print('fixture/oracle OK:', task['id'])

if __name__ == '__main__':
    selftest()
