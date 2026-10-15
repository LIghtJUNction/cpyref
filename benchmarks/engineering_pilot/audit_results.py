"""Post-run evidence audit. Incomplete usage is a lower bound, never a full cost.
Run: python audit_results.py PATH_TO_RUN_DIRECTORY
This does not execute a model, change tasks, or discard unsuccessful attempts.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
from tasks import make_tasks


def audit(root: Path) -> dict:
    expected = {(t['id'], arm) for t in make_tasks() for arm in ('native', 'cpyref')}
    rows, identities, receipts = [], [], []
    errors, incomplete = [], []
    for folder in sorted(root.glob('shard-*')):
        if not (folder / 'results.json').is_file():
            continue
        shard = json.loads((folder / 'results.json').read_text())
        env = json.loads((folder / 'environment.json').read_text())
        identities.append(tuple(env[k] for k in ('model_revision', 'model_sha256', 'llama_tag', 'llama_archive_sha256', 'docker_image')))
        if env['model_sha256'] != env['model_expected_sha256']:
            errors.append('Model digest does not match recorded expected digest')
        if 'sha256:' + env['llama_archive_sha256'] != env['llama_archive_expected_digest']:
            errors.append('Runtime digest does not match recorded expected digest')
        for row in shard:
            key = (row['task'], row['arm'])
            trace_path = folder / f'{key[0]}-{key[1]}' / 'trace.json'
            trace = json.loads(trace_path.read_text())
            incoming = outgoing = replies = shell_turns = 0
            cpyref_turns, protocol_errors, inference_errors = [], [], []
            for number, turn in enumerate(trace):
                response = turn.get('response')
                if response is None:
                    inference_errors.append({'turn': number, 'error': turn.get('error')})
                    continue
                usage = response.get('usage', {})
                if not all(isinstance(usage.get(k), int) for k in ('prompt_tokens', 'completion_tokens')):
                    errors.append(f'Missing backend usage: {key}, turn {number}')
                    continue
                incoming += usage['prompt_tokens']
                outgoing += usage['completion_tokens']
                replies += 1
                content = response['choices'][0]['message'].get('content') or ''
                action = None
                try:
                    action = json.loads(content)
                    if not isinstance(action, dict) or action.get('action') not in ('shell', 'done'):
                        raise ValueError('Unknown action')
                except (ValueError, TypeError):
                    protocol_errors.append(number)
                if 'tool_result' in turn:
                    shell_turns += 1
                    if isinstance(action, dict) and 'cpyref ' in str(action.get('command', '')):
                        cpyref_turns.append(number)
            if (incoming, outgoing, replies, shell_turns, len(cpyref_turns)) != (
                    row['prompt_tokens'], row['completion_tokens'], row['model_calls'],
                    row['tool_calls'], row['cpyref_command_calls']):
                errors.append(f'Trace totals differ from episode totals: {key}')
            if row['passed'] != row['acceptance']['passed']:
                errors.append(f'Acceptance flag mismatch: {key}')
            if inference_errors or not row['usage_complete']:
                incomplete.append({'task': key[0], 'arm': key[1], 'errors': inference_errors})
            receipt = dict(task=key[0], arm=key[1], requests=len(trace), responses=replies,
                           input_tokens_received=incoming, output_tokens_received=outgoing,
                           usage_complete=not inference_errors and row['usage_complete'],
                           passed=row['passed'], cpyref_shell_turns=cpyref_turns,
                           protocol_error_turns=protocol_errors,
                           elapsed_including_acceptance_seconds=row['elapsed_seconds']+row['acceptance_seconds'],
                           trace_sha256=hashlib.sha256(trace_path.read_bytes()).hexdigest())
            receipts.append(receipt)
            rows.append(row)
    actual = [(r['task'], r['arm']) for r in rows]
    if len(actual) != len(expected) or set(actual) != expected:
        errors.append('Missing or duplicate task-condition attempts')
    if len(set(identities)) != 1:
        errors.append('Missing or inconsistent model/runtime/container identities')
    totals = {}
    for arm in ('native', 'cpyref'):
        group = [r for r in receipts if r['arm'] == arm]
        inc = sum(r['input_tokens_received'] for r in group)
        out = sum(r['output_tokens_received'] for r in group)
        complete = len(group) == len(make_tasks()) and all(r['usage_complete'] for r in group)
        accepted = sum(r['passed'] for r in group)
        totals[arm] = dict(attempts=len(group), accepted_by_frozen_checks=accepted,
                           usage_complete_attempts=sum(r['usage_complete'] for r in group),
                           input_tokens_received=inc, output_tokens_received=out,
                           received_token_counts_are_lower_bounds=not complete,
                           full_input_tokens=inc if complete else None,
                           full_output_tokens=out if complete else None,
                           model_requests=sum(r['requests'] for r in group),
                           model_responses=sum(r['responses'] for r in group),
                           attempts_invoking_cpyref=sum(bool(r['cpyref_shell_turns']) for r in group),
                           accepted_attempts_invoking_cpyref=sum(r['passed'] and bool(r['cpyref_shell_turns']) for r in group),
                           sum_attempt_seconds_including_acceptance=sum(r['elapsed_including_acceptance_seconds'] for r in group),
                           full_tokens_per_accepted_task=(inc+out)/accepted if complete and accepted else None,
                           monetary_cost_usd=None)
    return dict(experiment_run='38671711888', expected_attempts=len(expected), observed_attempts=len(rows),
                trace_accounting_consistent=not errors, accounting_errors=errors,
                incomplete_inference=incomplete, full_cost_comparison_valid=not errors and not incomplete,
                efficacy_claim_supported=False,
                interpretation='Not an efficiency proof. Preserve failed attempts and missing token usage; do not infer tool benefit from a success without tool invocation.',
                totals=totals, receipts=receipts)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    root = parser.parse_args().root
    result = audit(root)
    (root/'AUDIT.json').write_text(json.dumps(result, ensure_ascii=False, indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k != 'receipts'}, ensure_ascii=False, indent=2))
    if result['accounting_errors']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
