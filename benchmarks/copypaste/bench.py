#!/usr/bin/env python3
"""CopyPaste Benchmark: deterministic tool trials, not simulated LLM trials.
Python >=3.11; no third-party packages. Only generated/public fixture data is used.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone

VERSION = "0.1.0"
ARMS = ("cpyref", "python_bytes", "inline_base64")
# Baselines are sent as executable Python -c programs, not a preinstalled rival tool.
PRELUDE = "import sys,json,re,os,hashlib,tempfile,pathlib; P=pathlib.Path\n"
REJECT = "def stop(n):\n print(json.dumps({'error':n}),file=sys.stderr); sys.exit(n)\n"
WRITER = '''
def put(p, x, force=False):
 p=P(p)
 if p.is_symlink() or (p.exists() and not p.is_file()): stop(5)
 if p.exists() and not force: stop(5)
 f=tempfile.NamedTemporaryFile(dir=p.parent,delete=False)
 try:
  f.write(x); f.flush(); os.fsync(f.fileno()); f.close()
  if force: os.replace(f.name,p)
  else: os.link(f.name,p)
 finally:
  f.close()
  if os.path.exists(f.name): os.unlink(f.name)
'''


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def js(value: object) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def line_bytes(data: bytes, first: int, last: int) -> bytes:
    # Golden oracle scans LF positions; baseline uses splitlines with LF-only regexp.
    starts = [0] + [i + 1 for i, b in enumerate(data) if b == 10 and i + 1 < len(data)]
    end = starts[last] if last < len(starts) else len(data)
    return data[starts[first - 1]:end]


@dataclass
class Task:
    id: str
    source: bytes
    wanted: bytes
    options: list[str] = field(default_factory=list)
    operation: str = "new"
    expected_exit: int = 0
    lifecycle: str = "normal"
    copies: int = 1
    origin: str = "generated-workflow"
    initial: bytes | None = None
    description: str = ""

    @property
    def staged(self) -> bool:
        return self.lifecycle != "normal"

    @property
    def composed(self) -> bool:
        return self.operation in ("insert", "replace_region", "append", "ambiguous_destination")


def tasks(source_root: Path, seed: int) -> list[Task]:
    rng = random.Random(seed)
    items: list[Task] = []
    def add(name, data, wanted=None, **kw):
        items.append(Task(name, data, data if wanted is None else wanted, **kw))
    for name, file, lines in [
        ("R01-license", "LICENSE.MIT", None),
        ("R02-chinese-doc", "docs/agent-contract.md", None),
        ("R03-config-lines", "Cargo.toml", (2, 18)),
        ("R04-rust-lines", "src/error.rs", (1, 24)),
    ]:
        data = (source_root / file).read_bytes()
        kw = {"origin": "repository:" + file, "description": "Copy " + file + (f" lines {lines}" if lines else " exactly")}
        if lines:
            add(name, data, line_bytes(data, *lines), options=["--lines", f"{lines[0]}:{lines[1]}"], **kw)
        else:
            add(name, data, **kw)
    readme = (source_root / "README.md").read_bytes()
    start, end = "## 基本用法".encode(), "\n## 查找".encode()
    if readme.count(start) != 1 or readme.count(end) != 1:
        raise ValueError("Real README section changed; revise the task, do not silently skip it")
    add("R05-readme-section", readme, readme[readme.index(start):readme.index(end)],
        options=["--from", start.decode(), "--to", end.decode(), "--exclude-end"], origin="repository:README.md")
    add("R06-readme-fanout10", readme, copies=10, origin="repository:README.md")
    url = ("https://example.invalid/download?value=" + "".join(rng.choice("aZ09_%=-") for _ in range(4096))).encode()
    add("S01-long-url", b'<a href="' + url + b'">file</a>', url,
        options=["--regex", 'href="(?P<url>[^"]+)"', "--group", "url"])
    add("S02-shell-characters", b'\'"`$(){}[]\\% ;\t  exact\r\n')
    add("S03-unicode-whitespace", ("汉字\t e\u0301 é 零宽\u200b 空格  \r\n" * 80).encode())
    add("S04-crlf-lines", b"head\r\n  first\r\n\tsecond\r\ntail\r\n", b"  first\r\n\tsecond\r\n", options=["--lines", "2:3"])
    add("S05-binary-256k", bytes(range(256)) * 1024)
    add("S06-no-final-newline", b"head\nlast", b"last", options=["--lines", "2"])
    data = b"[dev] timeout = 30 disabled\n[prod] timeout = 30 enabled\n"
    add("S07-context-disambiguation", data, b"timeout = 30", options=["--text", "timeout = 30", "--before", "[prod]", "--after", "enabled", "--context-lines", "0"])
    add("S08-multiline-markers", b"head BEGIN\r\n  body\r\nEND tail", b"\r\n  body\r\n", options=["--from", "BEGIN", "--to", "END", "--exclude-markers"])
    add("S09-overlap-ambiguity", b"banana", b"", options=["--text", "ana"], expected_exit=3)
    add("S10-no-match", b"present", b"", options=["--text", "missing"], expected_exit=2)
    add("S11-invalid-regex", b"content", b"", options=["--regex", "["], expected_exit=4)
    add("S12-empty-input", b"", expected_exit=8)
    add("S13-protect-existing", b"new bytes", initial=b"DO NOT OVERWRITE", expected_exit=5)
    add("S14-authorized-replace", b"replacement\r\n", initial=b"old bytes", operation="force")
    if os.name != "nt":
        add("S15-protect-symlink", b"new bytes", operation="symlink", expected_exit=5)
    for name, life, code in [
        ("S16-source-deleted", "delete", 0), ("S17-source-modified", "modify", 0),
        ("S18-same-project-child", "child", 0), ("S19-other-project", "other_project", 6),
        ("S20-other-namespace", "other_namespace", 6), ("S21-shared-projects", "shared", 0),
        ("S22-corrupt-snapshot", "corrupt", 7),
    ]:
        add(name, b"saved immutable bytes\r\n", lifecycle=life, expected_exit=code)
    add("S23-insert-before-anchor", b"inserted\r\n", operation="insert", initial=b"prefix\n<!-- HERE -->\nsuffix\n")
    add("S24-replace-region", b"replacement", operation="replace_region", initial=b"prefix[START]old[END]suffix")
    add("S25-append", b"APPEND\x00\r\n", operation="append", initial=b"unchanged-prefix")
    add("S26-ambiguous-destination", b"new", operation="ambiguous_destination", initial=b"<!-- HERE --><!-- HERE -->", expected_exit=3)
    add("S27-whole-1m", rng.randbytes(1024 * 1024))
    add("S28-whole-64b", rng.randbytes(64))
    add("S29-stdin", b"tool output\r\n" * 200, operation="stdin")
    return items


def selection(task: Task) -> str:
    """Generate a standalone strict baseline recipe from public selectors only."""
    # Boolean marker flags need parsing independent of positional pairs.
    opts = {}; i = 0
    while i < len(task.options):
        key = task.options[i]; i += 1
        if key in ("--exclude-end", "--exclude-markers"):
            opts[key] = True
        else:
            opts[key] = task.options[i]; i += 1
    code = PRELUDE + REJECT
    code += "d=sys.stdin.buffer.read()\n" if task.operation == "stdin" else "d=P('input.dat').read_bytes()\n"
    code += "if not d: stop(8)\n"
    if "--lines" in opts:
        bounds = [int(v) for v in str(opts["--lines"]).split(":")]
        first, last = bounds[0], bounds[-1]
        code += f"a=re.findall(b'[^\\n]*\\n|[^\\n]+$',d); x=b''.join(a[{first-1}:{last}])\n"
    elif "--regex" in opts:
        code += f"try: r=re.compile({opts['--regex'].encode()!r})\nexcept re.error: stop(4)\n"
        code += f"a=[m.group({opts.get('--group', 0)!r}) for m in r.finditer(d)]\n"
        code += "if not a: stop(2)\nif len(a)!=1: stop(3)\nx=a[0]\n"
    elif "--text" in opts:
        needle = str(opts["--text"]).encode()
        code += f"n={needle!r}; a=[m.start() for m in re.finditer(b'(?='+re.escape(n)+b')',d)]\n"
        if "--before" in opts:
            code += f"a=[i for i in a if {str(opts['--before']).encode()!r} in d[d.rfind(b'\\n',0,i)+1:i]]\n"
        if "--after" in opts:
            code += f"a=[i for i in a if {str(opts['--after']).encode()!r} in d[i+len(n):].split(b'\\n',1)[0]]\n"
        code += "if not a: stop(2)\nif len(a)!=1: stop(3)\nx=d[a[0]:a[0]+len(n)]\n"
    elif "--from" in opts:
        a, b = str(opts["--from"]).encode(), str(opts["--to"]).encode()
        code += f"a={a!r}; b={b!r}; spans=[]\n"
        code += "for m in re.finditer(re.escape(a),d):\n j=d.find(b,m.end())\n if j>=0: spans.append((m.start(),m.end(),j,j+len(b)))\n"
        code += "if not spans: stop(2)\nif len(spans)!=1: stop(3)\ns=spans[0]\n"
        left = 1 if opts.get("--exclude-markers") else 0
        right = 2 if opts.get("--exclude-markers") or opts.get("--exclude-end") else 3
        code += f"x=d[s[{left}]:s[{right}]]\n"
    else:
        code += "x=d\n"
    return code + "if not x: stop(8)\n"


def finish(task: Task) -> str:
    code = WRITER
    if task.composed:
        code += "p=P('out0.dat'); old=p.read_bytes()\n"
        if task.operation in ("insert", "ambiguous_destination"):
            code += "n=b'<!-- HERE -->'\nif old.count(n)!=1: stop(3)\nx=old.replace(n,x+n,1)\n"
        elif task.operation == "replace_region":
            code += "a=b'[START]'; b=b'[END]'\nif old.count(a)!=1 or old.count(b)!=1: stop(3)\ns=old.index(a)+len(a); e=old.index(b)\nif e<s: stop(3)\nx=old[:s]+x+old[e:]\n"
        else:
            code += "x=old+x\n"
        code += "put(p,x,True)\n"
    else:
        code += f"for i in range({task.copies}): put('out'+str(i)+'.dat',x,{task.operation == 'force'})\n"
    return code


class Recorder:
    def __init__(self, root: Path, binary: str):
        self.root, self.binary = root, binary
        self.events: list[dict] = []
        self.env = {k:v for k,v in os.environ.items() if not k.startswith('CPYREF_')}
        self.env.update(CPYREF_STORE=str(root / "store"), CPYREF_NO_CONFIG="true")

    def call(self, argv: list[str], cwd: Path, data: bytes | None = None) -> subprocess.CompletedProcess:
        started = time.perf_counter_ns()
        try:
            p = subprocess.run(argv, cwd=cwd, env=self.env, input=data,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
        except subprocess.TimeoutExpired as e:
            p = subprocess.CompletedProcess(argv, 124, e.stdout or b"", e.stderr or b"timeout")
        normalized = [a.replace(str(self.root), "<workspace>") for a in argv]
        normalized[0] = "cpyref" if argv[0] == self.binary else Path(argv[0]).name
        encoded = js(normalized).encode()
        self.events.append({"returncode":p.returncode, "wall_ms":(time.perf_counter_ns()-started)/1e6,
            "argv_bytes":len(encoded), "argv_sha256":sha(encoded), "argv":normalized,
            "stdin_bytes":len(data or b""), "stdout_bytes":len(p.stdout), "stderr_bytes":len(p.stderr),
            "stdout_sha256":sha(p.stdout), "stderr_preview":p.stderr[:240].decode(errors="replace")})
        return p


def expected_output(t: Task) -> bytes:
    if t.expected_exit:
        return t.initial or b""
    if t.operation == "insert":
        return t.initial.replace(b"<!-- HERE -->", t.wanted+b"<!-- HERE -->", 1)
    if t.operation == "replace_region":
        return b"prefix[START]"+t.wanted+b"[END]suffix"
    if t.operation == "append":
        return t.initial+t.wanted
    return t.wanted


def execute(t: Task, arm: str, binary: str) -> dict:
    with tempfile.TemporaryDirectory(prefix="cpbench-") as directory:
        root = Path(directory); a=root/"a"; b=root/"b"
        for p in (a,b,a/"child"):
            p.mkdir(parents=True, exist_ok=True)
        for p in (a,b): (p/".cpyref.toml").write_text("")
        (a/"input.dat").write_bytes(t.source)
        (b/"guard.dat").write_bytes(b"UNRELATED PROJECT MUST NOT CHANGE")
        if t.initial is not None: (a/"out0.dat").write_bytes(t.initial)
        if t.operation == "symlink": (a/"out0.dat").symlink_to(b/"guard.dat")
        rec=Recorder(root,binary); cwd=a; extra=[]
        if t.lifecycle == "shared": extra=["--scope","shared","--namespace","team"]
        started=time.perf_counter_ns(); last=None
        def cp(args, path=a, data=None, flags=None):
            return rec.call([binary]+(extra if flags is None else flags)+args,path,data)
        def py(code, path=a, data=None):
            return rec.call([sys.executable,"-I","-S","-c",code],path,data)
        try:
            if arm == "native_cp":
                for i in range(t.copies):
                    last=rec.call(["cp","input.dat",f"out{i}.dat"],a)
                    if last.returncode: break
            elif arm == "python_bytes" and not t.staged:
                last=py(selection(t)+finish(t), data=t.source if t.operation=="stdin" else None)
            else:
                if arm=="cpyref":
                    last=cp(["copy","-" if t.operation=="stdin" else "input.dat"]+t.options,
                            data=t.source if t.operation=="stdin" else None)
                    reference=last.stdout.decode(errors="replace").strip()
                    if last.returncode==0 and not re.fullmatch(r"c[0-9a-f]{12}",reference):
                        raise ValueError("Invalid success reference")
                elif arm=="inline_base64":
                    last=py(selection(t)+"import base64; sys.stdout.buffer.write(base64.b64encode(x))\n",
                            data=t.source if t.operation=="stdin" else None)
                    relay=last.stdout
                else:
                    (root/"state").mkdir()
                    save=str(root/"state"/"saved")
                    last=py(selection(t)+WRITER+f"put({save!r},x); put({(save+'.sha')!r},hashlib.sha256(x).hexdigest().encode())\n")
                if last.returncode==0:
                    if t.lifecycle=="delete": (a/"input.dat").unlink()
                    if t.lifecycle=="modify": (a/"input.dat").write_bytes(b"new source version")
                    if t.lifecycle=="child": cwd=a/"child"
                    if t.lifecycle in ("other_project","shared"): cwd=b
                    if t.lifecycle=="other_namespace": extra=["--namespace","other"]
                    if t.lifecycle=="corrupt":
                        if arm=="cpyref":
                            candidates=list((root/"store").rglob(reference+".ref"))
                            if len(candidates)!=1: raise ValueError("Cannot uniquely locate test snapshot")
                            q=candidates[0]; damaged=bytearray(q.read_bytes()); damaged[-1]^=1; q.write_bytes(damaged)
                        elif arm=="python_bytes": P=Path(save); P.write_bytes(b"damaged")
                    if arm=="cpyref":
                        for i in range(t.copies):
                            out="piece.dat" if t.composed else f"out{i}.dat"
                            last=cp(["paste",reference,"--output",out]+(["--force"] if t.operation=="force" else []),cwd)
                            if last.returncode: break
                        if last.returncode==0 and t.composed:
                            last=py(PRELUDE+REJECT+"x=P('piece.dat').read_bytes()\n"+finish(t),cwd)
                    elif arm=="inline_base64":
                        # Deliberate byte-relay ablation, not an LLM or isolation implementation.
                        last=py(PRELUDE+REJECT+"import base64; x=base64.b64decode(sys.stdin.buffer.read(),validate=True)\n"+finish(t),cwd,relay)
                    else:
                        retrieve=PRELUDE+REJECT
                        # Explicit producer/consumer context in this hand-written baseline.
                        path=save if t.lifecycle not in ("other_project","other_namespace") else save+"-other-context"
                        retrieve+=f"p=P({path!r})\nif not p.exists(): stop(6)\nx=p.read_bytes()\n"
                        retrieve+=f"if hashlib.sha256(x).hexdigest()!=P({(path+'.sha')!r}).read_text(): stop(7)\n"
                        last=py(retrieve+finish(t),cwd)
            elapsed=(time.perf_counter_ns()-started)/1e6
            observed_exit=last.returncode
            # Inline relay does not claim snapshot integrity or context enforcement.
            unsupported=(arm=="inline_base64" and t.lifecycle in ("other_project","other_namespace","corrupt"))
            output_ok=True; actual_hashes=[]
            for i in range(t.copies):
                p=cwd/f"out{i}.dat"
                should_exist=not t.expected_exit or t.initial is not None or t.operation=="symlink"
                if t.operation=="symlink":
                    output_ok &= p.is_symlink()
                elif should_exist:
                    output_ok &= p.is_file() and not p.is_symlink() and p.read_bytes()==expected_output(t)
                else:
                    output_ok &= not p.exists()
                actual_hashes.append(sha(p.read_bytes()) if p.is_file() and not p.is_symlink() else None)
            guard_ok=(b/"guard.dat").read_bytes()==b"UNRELATED PROJECT MUST NOT CHANGE"
            guard_ok &= all((p/".cpyref.toml").read_bytes()==b"" for p in (a,b))
            allowed={"a/.cpyref.toml","b/.cpyref.toml","b/guard.dat","a/input.dat"}
            if t.initial is not None or t.operation=="symlink": allowed.add("a/out0.dat")
            if not t.expected_exit:
                allowed.update(str((cwd/f"out{i}.dat").relative_to(root)) for i in range(t.copies))
            if arm=="cpyref" and t.composed: allowed.add("a/piece.dat")
            unexpected=[str(p.relative_to(root)) for project in (a,b) for p in project.rglob("*")
                        if (p.is_file() or p.is_symlink()) and str(p.relative_to(root)) not in allowed]
            diagnostic_ok=True
            if arm=="cpyref" and last.args[0]==binary and last.returncode:
                codes={2:{"NO_MATCH"},3:{"AMBIGUOUS"},4:{"INVALID_REGEX"},
                       5:{"OUTPUT_EXISTS","UNSAFE_OUTPUT"},6:{"REF_NOT_FOUND"},
                       7:{"CORRUPT_REF"},8:{"EMPTY_SELECTION"}}
                try:
                    diagnostic_ok=not last.stdout and json.loads(last.stderr)["error"]["code"] in codes.get(t.expected_exit,set())
                except (ValueError,KeyError,TypeError): diagnostic_ok=False
            source_ok=(not (a/"input.dat").exists()) if t.lifecycle=="delete" else (a/"input.dat").read_bytes()==(b"new source version" if t.lifecycle=="modify" else t.source)
            state_bytes=sum(p.stat().st_size for name in ("store","state") for p in (root/name).rglob("*") if p.is_file())
            passed=observed_exit==t.expected_exit and output_ok and guard_ok and source_ok and diagnostic_ok and not unexpected
            return {"task":t.id,"arm":arm,"origin":t.origin,"expected_exit":t.expected_exit,
                "observed_exit":observed_exit,"supported":not unsupported,"pass":passed if not unsupported else None,
                "status":"unsupported" if unsupported else ("pass" if passed else "fail"),
                "cpyref_native":not t.composed,"wall_ms":elapsed,"calls":len(rec.events),
                "argv_bytes":sum(e["argv_bytes"] for e in rec.events),
                "stdin_bytes":sum(e["stdin_bytes"] for e in rec.events),
                "stdout_bytes":sum(e["stdout_bytes"] for e in rec.events),
                "stderr_bytes":sum(e["stderr_bytes"] for e in rec.events),
                "state_bytes":state_bytes,"source_bytes":len(t.source),"selected_bytes":len(t.wanted),
                "output_ok":bool(output_ok),"guard_ok":guard_ok,"source_ok":source_ok,
                "diagnostic_ok":diagnostic_ok,"unexpected_files":unexpected,
                "expected_sha256":sha(expected_output(t)),"actual_sha256":actual_hashes,"events":rec.events}
        except Exception as e:
            return {"task":t.id,"arm":arm,"origin":t.origin,"expected_exit":t.expected_exit,
                "supported":True,"pass":False,"status":"harness_error","error":repr(e),
                "wall_ms":(time.perf_counter_ns()-started)/1e6,"events":rec.events}


def percentile(values: list[float], fraction: float) -> float:
    return sorted(values)[max(0,math.ceil(len(values)*fraction)-1)]


def report(rows: list[dict], meta: dict) -> tuple[dict,str]:
    supported=[r for r in rows if r["supported"]]
    by_case=[]
    for name in sorted({r["task"] for r in rows}):
        item={"task":name,"arms":{}}
        for arm in (*ARMS,"native_cp"):
            group=[r for r in supported if r["task"]==name and r["arm"]==arm]
            if not group: continue
            item["arms"][arm]={"n":len(group),"passed":sum(r["pass"] for r in group),
                "median_ms":statistics.median(r["wall_ms"] for r in group),
                "p95_ms":percentile([r["wall_ms"] for r in group],.95),
                "median_calls":statistics.median(r.get("calls",0) for r in group),
                "median_boundary_bytes":statistics.median(sum(r.get(k,0) for k in ("argv_bytes","stdin_bytes","stdout_bytes","stderr_bytes")) for r in group)}
        by_case.append(item)
    totals={arm:{"passed":sum(bool(r["pass"]) for r in supported if r["arm"]==arm),
                 "trials":sum(r["arm"]==arm for r in supported),
                 "unsupported":sum(r["arm"]==arm and not r["supported"] for r in rows)} for arm in (*ARMS,"native_cp")}
    summary={"metadata":meta,"totals":totals,"cases":by_case,
             "agent_trials":None,"model_tokens":None,"model_cost":None}
    lines=["# 复制粘贴 Benchmark：工具层实测", "",f"软件提交：`{meta['source_sha']}`。运行：{meta['run_url']}。", "",
        "**这是固定脚本驱动的真实进程测试，不是真实模型 A/B；Token、模型费用和模型任务成功率均未测量。**", "",
        "`python_bytes` 可直接用 Python 读写、筛选、临时快照及校验，不要求重写正文。`inline_base64` 是无损正文往返消融，不是模型重写模拟；不支持空间/损坏验证的项目单列。`native_cp` 只参加适用的整文件任务。", "",
        "计时包括启动进程、选择、复制和粘贴；不包括安装、造数据、验收、目录清理。每组独立目录，一次预热，之后按配对区组随机顺序重复。缓存未强制清空，不能代表冷盘性能。", "",
        "真实材料来自被测提交的 6 项仓库文件任务；S 开头为按实际工作流程构造的公开可复现数据，不是采集到的用户会话。插入、区段替换和追加由 cpyref 加通用 Python 写入步骤完成，不能算原生 paste 已支持。", "",
        "| 组 | 验收通过/支持的运行数 | 不支持 |", "|---|---:|---:|"]
    for arm,total in totals.items(): lines.append(f"| {arm} | {total['passed']}/{total['trials']} | {total['unsupported']} |")
    lines += ["", "拒绝歧义、越界空间、损坏和覆盖也属于正确验收；不把正确拒绝算复制失败。重复次数不是独立任务数量。", "",
        "| 任务 | cpyref 中位 ms | Python 中位 ms | inline 中位 ms | cp 中位 ms | cpyref/Python |", "|---|---:|---:|---:|---:|---:|"]
    for item in by_case:
        a=item['arms']; cells=[f"{a[v]['median_ms']:.3f}" if v in a else "—" for v in (*ARMS,"native_cp")]
        ratio=a['cpyref']['median_ms']/a['python_bytes']['median_ms']
        lines.append(f"| {item['task']} | {' | '.join(cells)} | {ratio:.2f} |")
    lines += ["", "比值小于 1 表示本次 cpyref 脚本流程更快；大于 1 表示 Python 流程更快。不可把不同语义和不同覆盖面的组汇成一个排行榜。", "",
        "`summary.json` 另含各任务 p95、进程调用数和边界字节数。边界字节数 = 规范化 argv JSON + stdin + stdout + stderr 的 UTF-8/原始字节长度，是本测试进程接口流量，不是模型 Token。", "",
        "局限：固定策略、单台 CI 机器、单次环境批次、未评估模型规划/工具发现/编写脚本/重试/付费、未测断电持久性和恶意同账号进程隔离。请先读设计文档，再解释结果。", ""]
    return summary,"\n".join(lines)


def main() -> None:
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--source",type=Path,required=True); p.add_argument("--binary",required=True)
    p.add_argument("--out",type=Path,required=True); p.add_argument("--repeats",type=int,default=20)
    p.add_argument("--seed",type=int,default=20261014); p.add_argument("--baseline-only",action="store_true")
    args=p.parse_args()
    if not 1<=args.repeats<=1000: p.error("repeats must be 1..1000")
    suite=tasks(args.source,args.seed); args.out.mkdir(parents=True,exist_ok=True)
    source_sha=subprocess.check_output(["git","-C",str(args.source),"rev-parse","HEAD"],text=True).strip()
    version="not run (baseline validation)" if args.baseline_only else subprocess.check_output([args.binary,"--version"],text=True).strip()
    meta={"benchmark_version":VERSION,"source_sha":source_sha,"cpyref_version":version,
        "binary_sha256":None if args.baseline_only else sha(Path(args.binary).read_bytes()),
        "harness_sha256":sha(Path(__file__).read_bytes()),"cargo_lock_sha256":sha((args.source/"Cargo.lock").read_bytes()),
        "utc":datetime.now(timezone.utc).isoformat(),"platform":platform.platform(),"python":sys.version,
        "cpu_count":os.cpu_count(),"seed":args.seed,"repeats":args.repeats,"warmup":1,
        "unique_tasks":len(suite),"run_url":os.environ.get("BENCH_RUN_URL","local"),
        "install_command":os.environ.get("BENCH_INSTALL_COMMAND","not recorded"),
        "agent_evaluation":"not_run","cache_policy":"warm; no cache eviction"}
    (args.out/"manifest.json").write_text(json.dumps({"metadata":meta,"tasks":[{
        "id":t.id,"origin":t.origin,"description":t.description,"source_sha256":sha(t.source),
        "selected_sha256":sha(t.wanted),"source_bytes":len(t.source),"selected_bytes":len(t.wanted),
        "options":t.options,"operation":t.operation,"lifecycle":t.lifecycle,"expected_exit":t.expected_exit,
        "copies":t.copies,"cpyref_native":not t.composed} for t in suite]},ensure_ascii=False,indent=2)+"\n")
    rows=[]; rng=random.Random(args.seed)
    with (args.out/"raw.jsonl").open("w",encoding="utf-8") as stream:
        for repeat in range(-1,args.repeats):
            ordered=suite[:]; rng.shuffle(ordered)
            for task in ordered:
                arms=list(ARMS[1:] if args.baseline_only else ARMS)
                if shutil.which("cp") and not task.options and not task.staged and task.operation=="new" and not task.expected_exit:
                    arms.append("native_cp")
                rng.shuffle(arms)
                for arm in arms:
                    row=execute(task,arm,args.binary); row.update(repeat=repeat,seed=args.seed)
                    if repeat>=0:
                        if repeat > 0:
                            for event in row["events"]: event.pop("argv", None)
                        rows.append(row); stream.write(js(row)+"\n"); stream.flush()
    if args.baseline_only:
        failed=[r for r in rows if r['supported'] and not r['pass']]
        print(js({"baseline_selfcheck_rows":len(rows),"failures":failed})); sys.exit(bool(failed))
    summary,markdown=report(rows,meta)
    (args.out/"summary.json").write_text(json.dumps(summary,ensure_ascii=False,indent=2)+"\n")
    (args.out/"REPORT.md").write_text(markdown)
    print(markdown)
    print("BENCH_TOTALS="+js(summary["totals"]))
    # A product failure is data, not a reason to suppress the report.
    if any(r['status']=='harness_error' for r in rows): sys.exit(2)


if __name__=="__main__": main()
