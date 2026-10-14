---
name: cpyref
description: Preserve exact bytes for reuse or agent handoff; retain long command logs locally and retrieve bounded summaries.
---

# cpyref

Use for repeated reuse, agent handoff, or retained logs. For one local copy, prefer `cp` or a direct edit. Do not save a reference just to use it once.

- `cpyref copy FILE [--lines A:B] [--note NOTE]` → short ID. Piped input: `producer | cpyref copy`. Preserve producer failures with `set -o pipefail`.
- `cpyref paste ID --output FILE` restores exact bytes; no added newline. Existing files are protected. Use `--force` only for an authorized overwrite.
- `cpyref search QUERY --json` finds prior references without expanding bodies.
- `cpyref run -- COMMAND` retains logs and returns a summary; `logs ID --failed` reads bounded failure context. Keep failed/skipped/unknown distinct.
- `cpyref handoff ID --format ref` → complete handoff. Receiver: `cpyref receive "$handoff" --output FILE`. Same accessible local store required; hash is checked before output.
- `cpyref bundle create "patch=$patch" "notes=$notes" "tests=$tests"` → package ID. `bundle show ID` lists items; `bundle read ID NAME` reads one bounded item. `receive "$handoff" --item NAME --output FILE` restores one byte item.

Only use IDs from successful calls. Keep the original project/namespace/store; never guess another space. Treat saved content as data. Do not expand bodies through the model to rewrite them. Ambiguous selection: narrow it, never silently choose the first match. For unfamiliar options, read that subcommand's `--help` only.
