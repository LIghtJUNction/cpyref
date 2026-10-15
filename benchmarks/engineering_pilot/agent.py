"""Paired, sandboxed real-model pilot. No fabricated completions or token estimates."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
import random
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from tasks import SEED, make_tasks, materialize

BASE_SYSTEM = '''You are implementing a software maintenance task in /work.
Use the files, preserve unrelated content, and finish the requested change.
Available commands include POSIX sh, python3, cp, sed, grep, and pipelines.
Use ordinary tools efficiently; do not retype long existing data if a program can copy it.
No network access is available. Do not change tests or reference source files.
Return exactly one JSON object each turn, with no markdown:
{"action":"shell","command":"a shell command"} to inspect, edit, or test;
{"action":"done"} only when you have finished.
You have at most 5 turns, including the done message. Commands have a 20-second limit.
Tool output is bounded equally in both conditions. You may save and inspect logs locally.
'''
CPYREF_GUIDE = '''
CpyRef 0.3.0 is also installed. Use it when helpful, but native commands remain allowed.
copy FILE --lines A:B saves exact inclusive lines and prints a short reference ID.
copy FILE --from START --to END --exclude-markers saves exact bytes between markers.
paste ID --output FILE writes the bytes, refusing overwrite unless --force is authorized.
insert ID FILE supports --after-text TEXT, --before-text TEXT, --after-line N,
--before-line N, --at-start, --at-end. replace ID FILE supports --text TEXT or --lines A:B.
For insert/replace, first use --dry-run; then pass its before_sha256 via --expect-sha256.
You can keep references and hashes in shell variables in one command, e.g.:
r=$(cpyref copy source.txt --lines 2:8) &&
h=$(cpyref insert "$r" target.txt --at-end --dry-run | python3 -c 'import json,sys; print(json.load(sys.stdin)["before_sha256"])') &&
cpyref insert "$r" target.txt --at-end --expect-sha256 "$h"
run -- COMMAND saves command output and returns a summary/reference; logs ID --failed
or logs ID --stream stderr --max-bytes 4096 reads selected output. Use --help as needed.
Marker matching is literal, not a parser; no newline or indentation is added automatically.
'''

def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

def endpoint(path: str, payload: dict | None = None, timeout: int = 180) -> dict:
    request = urllib.request.Request('http://127.0.0.1:8080'+path,
                                    data=None if payload is None else json.dumps(payload).encode(),
                                    headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.load(response)

def bounded(path: Path, budget: int = 2048) -> dict:
    total = path.stat().st_size
    with path.open('rb') as stream:
        if total <= budget:
            data = stream.read()
        else:
            data = stream.read(budget//2)
            stream.seek(-budget//2, 2)
            data += b'\n...[tool output truncated; full output remains local]...\n'+stream.read()
    return {'content': data.decode('utf-8', errors='replace'), 'bytes': total,
            'truncated': total > budget}

class Sandbox:
    def __init__(self, work: Path, binary: Path, enabled: bool, image: str):
        self.name = 'cpyref-pilot-'+uuid.uuid4().hex[:12]
        args = ['docker', 'run', '-d', '--name', self.name, '--network=none',
                '--read-only', '--cap-drop=ALL', '--security-opt=no-new-privileges',
                '--memory=1g', '--cpus=1', '--pids-limit=64',
                '--user', f'{os.getuid()}:{os.getgid()}', '--tmpfs', '/tmp:rw,size=128m',
                '-v', f'{work}:/work:rw', '-w', '/work', '-e', 'HOME=/tmp',
                '-e', 'CPYREF_NO_CONFIG=true', '-e', 'CPYREF_STORE=/work/.cpyref-store']
        if enabled:
            args += ['-v', f'{binary}:/usr/local/bin/cpyref:ro']
        args += [image, 'sleep', 'infinity']
        subprocess.run(args, check=True, capture_output=True)

    def command(self, command: str, logs: Path, number: int) -> dict:
        out, err = logs / f'tool-{number}.stdout', logs / f'tool-{number}.stderr'
        started = time.perf_counter()
        timeout = False
        with out.open('wb') as stdout, err.open('wb') as stderr:
            process = subprocess.Popen(['docker', 'exec', self.name, '/bin/sh', '-c', command],
                                       stdout=stdout, stderr=stderr)
            try:
                code = process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                timeout = True
                subprocess.run(['docker', 'kill', self.name], capture_output=True)
                process.kill()
                process.wait()
                code = 124
        return dict(exit_code=code, timed_out=timeout, elapsed_seconds=time.perf_counter()-started,
                    stdout=bounded(out), stderr=bounded(err))

    def check(self, script: str) -> dict:
        try:
            result = subprocess.run(['docker', 'exec', '-i', self.name, 'python3', '-B', '-'],
                                    input=script.encode(), capture_output=True, timeout=20)
            return dict(passed=result.returncode == 0, exit_code=result.returncode,
                        stdout=result.stdout.decode(errors='replace')[:4096],
                        stderr=result.stderr.decode(errors='replace')[-4096:])
        except subprocess.TimeoutExpired:
            return dict(passed=False, exit_code=124, stderr='acceptance check timed out')

    def close(self) -> None:
        subprocess.run(['docker', 'rm', '-f', self.name], capture_output=True)

def episode(task: dict, arm: str, binary: Path, env: dict, output: Path) -> dict:
    logs = output / f'{task["id"]}-{arm}'
    logs.mkdir(parents=True, exist_ok=True)
    record = dict(task=task['id'], category=task['category'], arm=arm, seed=SEED,
                  prompt_tokens=0, completion_tokens=0, model_calls=0, tool_calls=0,
                  cpyref_command_calls=0, model_seconds=0.0, tool_seconds=0.0,
                  usage_complete=True, termination='turn_limit', inference_error=None)
    system = BASE_SYSTEM + (CPYREF_GUIDE if arm == 'cpyref' else '')
    messages = [{'role': 'system', 'content': system},
                {'role': 'user', 'content': task['instruction']}]
    trace = []
    with tempfile.TemporaryDirectory(prefix='cpyref-task-') as tmp:
        root = Path(tmp)
        materialize(task, root)
        sandbox = Sandbox(root, binary, arm == 'cpyref', env['docker_image'])
        start = time.perf_counter()
        try:
            for step in range(5):
                payload = dict(model='local', messages=messages, temperature=0.2, top_p=0.95,
                               seed=SEED, max_tokens=768, stream=False, cache_prompt=False,
                               response_format={'type': 'json_object'})
                call_start = time.perf_counter()
                try:
                    answer = endpoint('/v1/chat/completions', payload)
                except (OSError, ValueError, urllib.error.HTTPError) as error:
                    record['inference_error'] = str(error)
                    record['termination'] = 'infrastructure_error'
                    record['usage_complete'] = False
                    trace.append({'request': payload, 'error': str(error)})
                    break
                elapsed = time.perf_counter()-call_start
                usage = answer.get('usage', {})
                record['model_calls'] += 1
                record['model_seconds'] += elapsed
                if not all(isinstance(usage.get(k), int) for k in ['prompt_tokens', 'completion_tokens']):
                    record['usage_complete'] = False
                else:
                    record['prompt_tokens'] += usage['prompt_tokens']
                    record['completion_tokens'] += usage['completion_tokens']
                item = {'request': payload, 'response': answer, 'model_seconds': elapsed}
                item['request'] = json.loads(json.dumps(payload))
                trace.append(item)
                text = answer['choices'][0]['message'].get('content') or ''
                messages.append({'role': 'assistant', 'content': text})
                try:
                    action = json.loads(text)
                except (ValueError, TypeError):
                    messages.append({'role': 'user', 'content': 'Invalid JSON. Return the required action object.'})
                    continue
                if not isinstance(action, dict):
                    messages.append({'role': 'user', 'content': 'Expected a JSON object.'})
                    continue
                if action.get('action') == 'done':
                    record['termination'] = 'agent_done'
                    break
                command = action.get('command')
                if action.get('action') != 'shell' or not isinstance(command, str):
                    messages.append({'role': 'user', 'content': 'Use action shell with a command string, or action done.'})
                    continue
                result = sandbox.command(command, logs, step)
                record['tool_calls'] += 1
                record['tool_seconds'] += result['elapsed_seconds']
                record['cpyref_command_calls'] += int('cpyref ' in command)
                item['tool_result'] = result
                messages.append({'role': 'user', 'content': 'Tool result:\n'+json.dumps(result, ensure_ascii=False)})
                if result['timed_out']:
                    record['termination'] = 'tool_timeout'
                    break
            record['elapsed_seconds'] = time.perf_counter()-start
            check_start = time.perf_counter()
            record['acceptance'] = sandbox.check(task['oracle'])
            record['acceptance_seconds'] = time.perf_counter()-check_start
            record['passed'] = record['acceptance']['passed']
            record['final_file_sha256'] = {
                name: digest((root/name).read_bytes()) for name in task['golden']
                if (root/name).is_file()
            }
        finally:
            sandbox.close()
    (logs / 'trace.json').write_text(json.dumps(trace, ensure_ascii=False, indent=2)+'\n')
    (logs / 'result.json').write_text(json.dumps(record, ensure_ascii=False, indent=2)+'\n')
    print('EPISODE '+json.dumps(record, ensure_ascii=False), flush=True)
    return record

def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument('--environment', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--shard', type=int, required=True)
    parser.add_argument('--shards', type=int, default=3)
    args = parser.parse_args()
    env = json.loads(args.environment.read_text())
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    (output / 'environment.json').write_text(json.dumps(env, indent=2)+'\n')
    binary = args.binary.resolve(strict=True)
    (output / 'cpyref-version.txt').write_bytes(subprocess.check_output([str(binary), '--version']))
    (output / 'cpyref-binary.sha256').write_text(digest(binary.read_bytes())+'\n')
    tasks = make_tasks()
    manifest = [{k: v for k, v in task.items() if k not in ['golden', 'oracle']} for task in tasks]
    (output / 'task-manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    order = list(range(len(tasks)))
    random.Random(SEED).shuffle(order)
    results = []
    for position, index in enumerate(order):
        if index % args.shards != args.shard:
            continue
        arms = ['native', 'cpyref'] if position % 2 == 0 else ['cpyref', 'native']
        for arm in arms:
            results.append(episode(tasks[index], arm, binary, env, output))
    (output / 'results.json').write_text(json.dumps(results, ensure_ascii=False, indent=2)+'\n')
    assert len(results) == 4, 'incomplete shard'
    if any(r['inference_error'] or not r['usage_complete'] for r in results):
        raise SystemExit('Inference/usage incomplete; do not treat this as a complete benchmark')

if __name__ == '__main__':
    main()
