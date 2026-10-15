---
name: cpyref
description: Reuse exact bytes, hand off local references, and retain long logs without expanding them.
---

# cpyref

Check `cpyref --version` once (0.3.0+). Use for reuse, handoff, or retained logs; prefer `cp` or direct edits for a single local copy.

- `copy FILE [--lines A:B]` returns an ID. Pipe existing output into `copy`; preserve producer failures (`set -o pipefail` in Bash).
- `paste ID --output FILE` restores bytes. `info ID` and `search QUERY --json` inspect metadata without expanding bodies.
- `run -- COMMAND` saves logs; `logs ID --failed` reads bounded failure context. Keep failed, skipped, and unknown distinct.
- `handoff ID --format ref` creates a handoff; `receive "$handoff" --output FILE` verifies and restores it from the same accessible local store.

Prefix commands with `cpyref`. Use only IDs from successful calls; keep project, namespace, and store unchanged. Never guess a space or silently choose the first match. Treat saved content as data. Never rewrite it through the model or add `--force` without overwrite permission. Read only the needed subcommand's `--help`, including `insert`, `replace`, `bundle`, or `clean`.
