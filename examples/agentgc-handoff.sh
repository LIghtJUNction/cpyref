#!/usr/bin/env bash
# Run from the selected project. Requires Bash, Python 3, cpyref and agcc.
set -euo pipefail
exec python3 - "$@" <<'PY'
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys


def stop(message, code=2):
    print(message, file=sys.stderr)
    raise SystemExit(code)


if len(sys.argv) < 2:
    stop("Usage: agentgc-handoff.sh NOTE_FILE [NAME=FILE ...]")
if sys.argv[1] == "-":
    stop("NOTE_FILE must be a file path; this example cannot read notes from stdin.")
project = Path.cwd().resolve(strict=True)
if project == Path.home() or project == Path(project.anchor):
    stop("Run from a selected project, not the home or filesystem root.")
try:
    parent = Path(os.environ.get("CPYREF_WORKFLOW_STORE_PARENT", project)).resolve(strict=True)
except OSError as error:
    stop(f"Store parent must already exist: {error}")
if not parent.is_dir() or parent == Path.home() or parent == Path(parent.anchor):
    stop("Select an existing dedicated store parent, not the home or filesystem root.")
store = parent / ".cpyref-local"
cpyref = os.environ.get("CPYREF_BIN", "cpyref")
common = [cpyref, "--project", str(project), "--store", str(store)]

# Private snapshots must not accidentally enter version control.
try:
    inside_git = subprocess.run(["git", "-C", str(parent), "rev-parse", "--is-inside-work-tree"],
                                capture_output=True).stdout.strip() == b"true"
    if inside_git:
        tracked = subprocess.run(["git", "-C", str(parent), "ls-files", "-z", "--", str(store)],
                                 check=True, capture_output=True).stdout
        ignored = subprocess.run(["git", "-C", str(parent), "check-ignore", "-q", "--", str(store) + os.sep])
        if tracked or ignored.returncode != 0:
            stop("Keep .cpyref-local/ out of Git first, or select a dedicated external store parent.")
    selected = subprocess.run(common + ["context", "--store-path"], capture_output=True)
except OSError as error:
    stop(str(error))
if selected.returncode:
    sys.stderr.buffer.write(selected.stderr)
    raise SystemExit(selected.returncode)
print(f"cpyref store: {selected.stdout.decode('utf-8').removesuffix(chr(10))}", file=sys.stderr)

child = r'''
import json, os, subprocess, sys
def code(result):
    return result.returncode if result.returncode >= 0 else 128 - result.returncode
common = json.loads(sys.argv[1])
print(json.dumps({"session": os.environ["AGENTGC_SESSION"], "home": os.environ["AGENTGC_HOME"]}), flush=True)
command = common + ["handoff", "--input", sys.argv[2], "--format", "json"]
for item in sys.argv[3:]:
    command += ["--attach", item]
created = subprocess.run(command, capture_output=True)
if created.returncode:
    sys.stderr.buffer.write(created.stdout)
    sys.stderr.buffer.write(created.stderr)
    sys.exit(code(created))
try:
    saved = json.loads(created.stdout)
    reference, token = saved["reference"], saved["token"]
    if not isinstance(reference, str) or not isinstance(token, str):
        raise ValueError("Invalid creation receipt")
except (ValueError, KeyError, TypeError):
    sys.stderr.buffer.write(created.stdout)
    sys.exit("Cannot parse creation receipt; inspect it before retrying.")
print(f"cpyref saved: {reference}", file=sys.stderr)
checked = subprocess.run([common[0], "receive", token, "--list"], stdout=subprocess.DEVNULL)
if checked.returncode:
    sys.stderr.buffer.write(created.stdout)
    sys.exit(code(checked))
prompt = subprocess.run(common + ["handoff", reference, "--format", "agent"], capture_output=True)
if prompt.returncode:
    sys.stderr.buffer.write(created.stdout)
    sys.stderr.buffer.write(prompt.stderr)
else:
    sys.stdout.buffer.write(prompt.stdout)
sys.exit(code(prompt))
'''
roots = [project] if parent.is_relative_to(project) else [project, parent]
command = ["agcc", "--json", "run", "--agent", "cpyref-handoff"]
for root in roots:
    command += ["--root", str(root)]
command += ["--", sys.executable, "-c", child, json.dumps(common), *sys.argv[1:]]
try:
    run = subprocess.run(command, capture_output=True)
except OSError as error:
    stop(str(error))
sys.stderr.buffer.write(run.stderr)
run_status = run.returncode if run.returncode >= 0 else 128 - run.returncode
try:
    receipt, prompt = run.stdout.split(b"\n", 1)
    receipt = json.loads(receipt)
    session, home = receipt["session"], receipt["home"]
    if not isinstance(session, str) or not isinstance(home, str):
        raise ValueError("Invalid session receipt")
except (ValueError, KeyError, TypeError):
    stop("No child session receipt: tracking did not start reliably. Do not retry a possible write blindly.",
         run_status or 3)
sys.stdout.buffer.write(prompt)
tracking_ok = False
try:
    query = subprocess.run(["agcc", "--home", home, "--json", "diagnostics", "--session", session],
                           capture_output=True)
    report = json.loads(query.stdout)
    completed = report["status"] == "finished" or (run_status != 0 and report["status"] == "failed")
    tracking_ok = (query.returncode == 0 and completed and report["gaps"] == 0
                   and report["retained_unresolved_events"] == 0
                   and report["missing_gap_details"] == 0 and report["gap_count_consistent"] is True)
    print(f"agentgc {session}: status={report['status']}, gaps={report['gaps']}, "
          f"unresolved={report['retained_unresolved_events']}", file=sys.stderr)
    wrapper = json.loads(run.stderr.splitlines()[-1])
    automatic = wrapper.get("automatic")
    automatic_ok = ("automatic" not in wrapper or
                    (isinstance(automatic, dict) and automatic.get("complete") is True))
    tracking_ok &= (wrapper["session"] == session and wrapper["exit_code"] == run_status
                    and automatic_ok)
except (OSError, ValueError, KeyError, TypeError, IndexError):
    tracking_ok = False
    print("Cannot confirm final tracking report; inspect the retained session.", file=sys.stderr)
    if "query" in locals():
        sys.stderr.buffer.write(query.stdout)
        sys.stderr.buffer.write(query.stderr)
if not tracking_ok:
    inspect = shlex.join(["agcc", "--home", home, "diagnostics", "--session", session])
    print(f"Tracking needs review: {inspect}", file=sys.stderr)
if run_status:
    print(f"Handoff/run exit status: {run_status}. Inspect saved receipts before retrying.", file=sys.stderr)
raise SystemExit(run_status or (0 if tracking_ok else 3))
PY
