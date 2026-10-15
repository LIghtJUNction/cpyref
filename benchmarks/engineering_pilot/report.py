"""Audit all predeclared episodes, then summarize without dropping failed tasks."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
from tasks import make_tasks

def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument('root', type=Path)
    root = parser.parse_args().root
    rows, environments = [], []
    for path in sorted(root.glob('shard-*/results.json')):
        rows.extend(json.loads(path.read_text()))
        environments.append(json.loads((path.parent/'environment.json').read_text()))
    expected = {(t['id'], arm) for t in make_tasks() for arm in ['native', 'cpyref']}
    actual = [(r['task'], r['arm']) for r in rows]
    errors = []
    if set(actual) != expected or len(actual) != len(expected):
        errors.append('Missing or duplicate predeclared episodes')
    for key in ['model_sha256', 'model_revision', 'llama_tag', 'llama_archive_sha256', 'docker_image']:
        if len({e.get(key) for e in environments}) != 1:
            errors.append('Inconsistent or missing environment: '+key)
    for row in rows:
        trace_path = next(root.glob(f'shard-*/{row["task"]}-{row["arm"]}/trace.json'))
        trace = json.loads(trace_path.read_text())
        in_tokens = out_tokens = calls = 0
        for turn in trace:
            if 'response' not in turn:
                continue
            usage = turn['response'].get('usage', {})
            if not all(isinstance(usage.get(k), int) for k in ['prompt_tokens', 'completion_tokens']):
                errors.append('Missing API usage: '+row['task']+'/'+row['arm'])
                continue
            in_tokens += usage['prompt_tokens']
            out_tokens += usage['completion_tokens']
            calls += 1
        if (in_tokens, out_tokens, calls) != (row['prompt_tokens'], row['completion_tokens'], row['model_calls']):
            errors.append('Trace/summary mismatch: '+row['task']+'/'+row['arm'])
        if row['inference_error'] or not row['usage_complete']:
            errors.append('Incomplete inference: '+row['task']+'/'+row['arm'])
    audit = {'expected_episodes': len(expected), 'observed_episodes': len(rows),
             'valid_complete_pilot': not errors, 'errors': errors}
    totals = {}
    for arm in ['native', 'cpyref']:
        group = [r for r in rows if r['arm'] == arm]
        success = sum(r['passed'] for r in group)
        incoming = sum(r['prompt_tokens'] for r in group)
        outgoing = sum(r['completion_tokens'] for r in group)
        elapsed = sum(r['elapsed_seconds']+r['acceptance_seconds'] for r in group)
        totals[arm] = dict(attempts=len(group), successes=success, prompt_tokens=incoming,
                           completion_tokens=outgoing, total_tokens=incoming+outgoing,
                           elapsed_seconds=elapsed, tool_calls=sum(r['tool_calls'] for r in group),
                           episodes_using_cpyref=sum(r['cpyref_command_calls'] > 0 for r in group),
                           tokens_per_success=(incoming+outgoing)/success if success else None,
                           seconds_per_success=elapsed/success if success else None,
                           price_ratio_sensitivity={str(k): {'units': incoming+k*outgoing,
                                'units_per_success': (incoming+k*outgoing)/success if success else None}
                                for k in [1,4,10]})
    summary = dict(audit=audit, totals=totals, model='Qwen3-4B-Instruct-2507 Q4_K_M',
                   price_ratio_note='Input tokens + R * output tokens; assumed R, not a currency or bill.',
                   scope='Six synthetic executable maintenance tasks; one seed; local CPU model; no population-wide proof.',
                   api_billed_cost_usd=0, compute_cost_usd=None)
    (root/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
    (root/'raw-results.json').write_text(json.dumps(rows, ensure_ascii=False, indent=2)+'\n')
    lines = ['# CpyRef engineering pilot — measured results', '',
             '**Scope:** six constructed, executable maintenance tasks. These are not mined real-world issues.',
             'Real local-model decisions, commands, responses, and backend token usage are retained. No completions are scripted.',
             'Model: Qwen3-4B-Instruct-2507, Q4_K_M. One seed (17), five-turn maximum. This is a pilot, not general proof.', '',
             f'Complete-data audit: **{"PASS" if not errors else "FAIL"}** ({len(rows)}/{len(expected)} episodes).', '',
             '| Condition | Passed | Input tokens | Output tokens | Time incl. acceptance (s) | Tool calls | Used CpyRef |',
             '|---|---:|---:|---:|---:|---:|---:|']
    for arm, total in totals.items():
        lines.append(f'| {arm} | {total["successes"]}/{total["attempts"]} | {total["prompt_tokens"]} | {total["completion_tokens"]} | {total["elapsed_seconds"]:.2f} | {total["tool_calls"]} | {total["episodes_using_cpyref"]} |')
    lines += ['', 'All failed attempts remain in token/time totals. Tokens per success also charge failed attempts.', '',
              '| Task | Native pass | CpyRef pass | Native input/output | CpyRef input/output | Native/CpyRef seconds |',
              '|---|---|---|---:|---:|---:|']
    lookup = {(r['task'], r['arm']): r for r in rows}
    for task in make_tasks():
        a, b = lookup.get((task['id'],'native')), lookup.get((task['id'],'cpyref'))
        if a and b:
            lines.append(f'| {task["id"]} | {a["passed"]} | {b["passed"]} | {a["prompt_tokens"]}/{a["completion_tokens"]} | {b["prompt_tokens"]}/{b["completion_tokens"]} | {a["elapsed_seconds"]+a["acceptance_seconds"]:.2f}/{b["elapsed_seconds"]+b["acceptance_seconds"]:.2f} |')
    lines += ['', '## Interpretation rules', '',
              '- Treat success, token use, and elapsed time as separate outcomes. Cheap failures are not efficiency gains.',
              '- The native condition retains Python, cp, sed, grep, and pipes. CpyRef is optional in the treatment.',
              '- Treatment-specific help is included in every actual model request and its token count.',
              '- Latency includes local CPU inference and Docker command execution, not model download/build/startup.',
              '- API fees are zero for this local run. Compute is not free by definition; no electricity/cloud price was measured.',
              '- The 1:1, 1:4, and 1:10 input/output price-ratio cases are sensitivity calculations, not observed invoices.',
              '- No cloud-model pricing or tokenizer can be inferred from this one quantized local model.',
              '- Six convenience-selected tasks and one seed cannot establish broad software-engineering superiority.',
              '- No unsuccessful model response is silently rerun or excluded. Infrastructure problems invalidate completeness.', '']
    if errors:
        lines += ['## Audit errors', '']+['- '+e for e in errors]
    (root/'REPORT.md').write_text('\n'.join(lines)+'\n')
    print(json.dumps(summary, indent=2))
    if errors:
        raise SystemExit('Incomplete pilot; inspect preserved artifacts instead of claiming a win')

if __name__ == '__main__':
    main()
