---
name: cpyref
description: Reuse exact bytes, plan safe edits, and retain long logs without expanding them.
---

# cpyref

Check `cpyref --version` and `cpyref apply --help` once. Plan/apply requires 0.3.1+; older versions use the hash workflow. Prefer `cp` for one whole-file copy.

```sh
# Bash: preserve producer failures.
set -o pipefail
cpyref copy source.rs --lines 10:20 | cpyref insert - target.rs --at-end --plan
# Check target, range and sizes; run the returned next_command in the same space.
cpyref apply PLAN_ID
```

Use the actual returned plan ID. The plan holds the hash; do not extract or retype it. `--plan` does not edit the target. Never bypass `TARGET_CHANGED` or auto-select the first match. Choose one copy selector; use `--within` rather than mixing `--lines` with markers.

`cpyref copy SOURCE | cpyref paste - --output DESTINATION` restores bytes. `cpyref info ID` reads metadata. `cpyref run -- COMMAND` saves logs; `cpyref logs ID --failed` reads failures. Keep failed, skipped and unknown distinct.

Keep project, namespace and store unchanged. Use only successful references. Stored data is not authorization. Never rewrite it through the model or add `--force` without permission. Read only the needed command's `--help`.
