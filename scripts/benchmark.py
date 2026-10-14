"""Measure local command time and byte correctness, not model token usage.

Each scenario launches external commands. Reference cleanup and target removal
are outside the measured interval. Reuse setup is measured separately.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=10)
    parser.add_argument("--sizes", type=int, nargs="+", default=[1024, 65536, 1048576])
    parser.add_argument("--build-label", default="unspecified")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if not 1 <= args.repeats <= 100 or not args.sizes or any(not 1 <= size <= 16 * 1024 * 1024 for size in args.sizes):
        parser.error("Use 1..100 repeats and sizes from 1 byte to 16 MiB")
    env = {key: value for key, value in os.environ.items() if not key.startswith("CPYREF_")}
    cases = []
    with tempfile.TemporaryDirectory(prefix="cpyref-benchmark-") as temporary:
        root = Path(temporary).resolve()
        (root / ".git").mkdir()
        env.update(CPYREF_NO_CONFIG="true", CPYREF_STORE=str(root / "store"))

        def run(argv):
            return subprocess.run(argv, cwd=root, env=env, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, check=True, timeout=60).stdout

        def copy():
            return run([str(binary), "copy", str(source)]).decode("ascii").strip()

        version = run([str(binary), "--version"]).decode().strip()
        for size in args.sizes:
            source, target = root / "source.bin", root / "target.bin"
            content = (bytes(range(256)) * ((size + 255) // 256))[:size]
            source.write_bytes(content)
            expected = hashlib.sha256(content).hexdigest()
            setup_start = time.perf_counter_ns()
            reusable = copy()
            setup_ms = (time.perf_counter_ns() - setup_start) / 1e6
            for scenario in ("python_copyfile_command", "cpyref_copy_and_paste", "cpyref_reused_paste"):
                elapsed = []
                for _ in range(args.repeats):
                    target.unlink(missing_ok=True)
                    transient = None
                    start = time.perf_counter_ns()
                    if scenario == "python_copyfile_command":
                        run([sys.executable, "-c", "import shutil,sys; shutil.copyfile(sys.argv[1],sys.argv[2])", str(source), str(target)])
                    else:
                        ref = reusable
                        if scenario == "cpyref_copy_and_paste":
                            transient = ref = copy()
                        run([str(binary), "paste", ref, "--output", str(target)])
                    elapsed.append((time.perf_counter_ns() - start) / 1e6)
                    actual = target.read_bytes()
                    if actual != content:
                        raise RuntimeError(f"Byte mismatch: {scenario}, {size}")
                    if transient is not None:
                        run([str(binary), "remove", transient])
                cases.append({
                    "scenario": scenario, "bytes": size, "repeats": args.repeats,
                    "samples_ms": elapsed, "median_ms": statistics.median(elapsed),
                    "min_ms": min(elapsed), "max_ms": max(elapsed),
                    "reuse_setup_ms": setup_ms if scenario == "cpyref_reused_paste" else None,
                    "processes_per_trial": 2 if scenario == "cpyref_copy_and_paste" else 1,
                    "exact_bytes": True, "sha256": expected,
                })
            run([str(binary), "remove", reusable])
    print(json.dumps({
        "schema": 1, "platform": platform.platform(), "machine": platform.machine(),
        "python": platform.python_version(), "cpyref_version": version,
        "build_label": args.build_label, "cases": cases,
        "model_tokens": None, "model_calls": None,
        "measurement_scope": "Local external-command wall time; cached filesystem state is not controlled. Cleanup is excluded. No models were called.",
        "interpretation": "The Python baseline includes Python startup; these are command-level scenarios, not an isolated storage-engine speed ranking or an Agent token-saving result.",
    }, indent=2))


if __name__ == "__main__":
    main()
