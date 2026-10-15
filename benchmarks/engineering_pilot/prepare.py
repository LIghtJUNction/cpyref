"""Download public inference artifacts and record immutable identities.
No private credentials, paid APIs, or remote coding agent are used.
"""
from __future__ import annotations
import hashlib
import json
import os
import platform
import re
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

REPO = 'bartowski/Qwen_Qwen3-4B-Instruct-2507-GGUF'
MODEL_FILE = 'Qwen_Qwen3-4B-Instruct-2507-Q4_K_M.gguf'

def get_json(url: str) -> dict:
    request = urllib.request.Request(url, headers={'User-Agent': 'cpyref-engineering-pilot'})
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)

def sha(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def download(url: str, path: Path) -> None:
    subprocess.run(['curl', '--fail', '--location', '--retry', '3', '--max-time', '600', '--output', str(path), url], check=True)

def main() -> None:
    root = Path(sys.argv[1]).resolve()
    root.mkdir(parents=True, exist_ok=True)
    release = get_json('https://api.github.com/repos/ggml-org/llama.cpp/releases/latest')
    assets = [a for a in release['assets'] if re.fullmatch(r'llama-.*-bin-ubuntu-x64.tar.gz', a['name'])]
    if len(assets) != 1:
        raise RuntimeError('Cannot identify a unique official llama.cpp Ubuntu CPU artifact')
    asset = assets[0]
    archive = root / asset['name']
    download(asset['browser_download_url'], archive)
    artifact_sha = sha(archive)
    if asset.get('digest') and asset['digest'] != 'sha256:' + artifact_sha:
        raise RuntimeError('llama.cpp artifact digest mismatch')
    with tarfile.open(archive) as package:
        package.extractall(root / 'llama', filter='data')
    candidates = [p for p in (root / 'llama').rglob('llama-server') if p.is_file()]
    if len(candidates) != 1:
        raise RuntimeError('Missing or ambiguous llama-server executable')
    server = candidates[0]
    server.chmod(server.stat().st_mode | 0o111)
    model_info = get_json(f'https://huggingface.co/api/models/{REPO}?blobs=true')
    revision = model_info['sha']
    model_entry = next(x for x in model_info['siblings'] if x['rfilename'] == MODEL_FILE)
    expected = model_entry.get('lfs', {}).get('sha256')
    model = root / MODEL_FILE
    download(f'https://huggingface.co/{REPO}/resolve/{revision}/{MODEL_FILE}?download=true', model)
    model_sha = sha(model)
    if expected and expected != model_sha:
        raise RuntimeError('Model artifact digest mismatch')
    subprocess.run(['docker', 'pull', 'python:3.12-slim'], check=True)
    image = subprocess.check_output(['docker', 'image', 'inspect', 'python:3.12-slim', '--format', '{{.Id}}'], text=True).strip()
    manifest = dict(model_repository=REPO, model_revision=revision, model_file=MODEL_FILE,
                    model_sha256=model_sha, model_path=str(model), model_expected_sha256=expected,
                    llama_tag=release['tag_name'], llama_archive_sha256=artifact_sha,
                    llama_archive_expected_digest=asset.get('digest'), server=str(server),
                    docker_image=image, platform=platform.platform(), cpu_count=os.cpu_count(),
                    python=sys.version, model_is_quantized=True,
                    api_billed_cost_usd=0, compute_cost_usd=None,
                    note='Local CPU inference. Zero API charge is not zero compute cost.')
    (root / 'environment.json').write_text(json.dumps(manifest, indent=2)+'\n')
    print(json.dumps(manifest, indent=2))

if __name__ == '__main__':
    main()
