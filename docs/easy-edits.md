# Copy, plan, apply

These commands require a source build with `insert --plan` and `apply`.
Existing released binaries may not have them. Check `cpyref apply --help` once.
The existing `--dry-run` / `--expect-sha256` route is unchanged.

## Reuse bytes without handling an ID or hash by hand

```sh
# Bash: also preserve the producer's failure status.
set -o pipefail
cpyref copy source.rs --lines 10:20 | cpyref insert - target.rs --at-end --plan
# Inspect the reported target, range and sizes. Use the actual returned plan ID.
cpyref apply PLAN_ID
```

The first command does not edit the target. It saves a plan and returns JSON with
`plan`, `applied=false`, and a complete `next_command`. Keep the same project,
namespace and store, including any explicit global flags, when running that command.
A plan stores the selection, source reference hash and target hash. `apply` checks
these again under the existing target lock. A changed target returns `TARGET_CHANGED`;
do not create and apply a fresh plan automatically just to bypass the check.

```sh
cpyref copy config-a.txt --text 'timeout = 30' | cpyref replace - config-b.txt --text 'timeout = 10' --plan
cpyref apply PLAN_ID --dry-run
cpyref apply PLAN_ID
```

For already-authorized, fully determined edits, a shell can pass the whole plan
result without a model round trip: append `| cpyref apply -` to the planning command.
This does not provide a separate review pause; do not use it when review is required.
The hash checks are still enforced. The plan input is data, never a shell program.

## Pipe references, not bodies

```sh
cpyref copy input.bin | cpyref paste - --output output.bin
cpyref copy input.bin --json | cpyref paste - --output output.bin
```

`-` accepts one full ID, or the matching successful JSON response (up to 64 KiB).
It rejects empty input, failed responses, multiple IDs, payloads and mismatched JSON
spaces. It does not search other projects. Interactive reference input is refused.
A pipe must reach EOF; producer processes that never close stdout need their own timeout.
Binary bytes stay in the store; only text IDs or JSON pass through the shell.

`paste`, `insert`, `replace`, `apply`, `info`, `remove`, `verify`, `logs`, and `handoff`
accept this reference input. `apply -` accepts a plan response; the other commands
accept a copy response. Plain IDs are always interpreted in the current space.

## Keep selections explicit

Choose one source selector: lines, exact text, regex, or start/end markers.
Do not combine `--lines` with `--from` / `--to`; use `--within` to limit a marker search.
Multiple matches still fail. No mode silently selects the first match, changes
indentation, adds a newline, or guesses function boundaries.

Plans use the current space's existing `list`, `info`, `remove`, `stats`, and `clean`
commands. Each plan occupies one extra entry, depends on its source reference,
and cannot be applied from another space. Delete the plan before its source, or
select both for an authorized cleanup. Receiving a plan only shows its saved preview.
It never executes it. Plans are retained after application; a repeat normally fails
because the target hash changed. They are not one-use tokens: unchanged or restored
original bytes can satisfy the same hash again. Old binaries do not understand the
new record type; do not mix versions against stores containing edit plans.

The existing edit size limits, ordinary-permission handling, path checks and
cooperating-writer lock still apply. Plans do not add a filesystem-wide transaction,
secret storage, automatic undo, or protection from malicious same-account writers.
