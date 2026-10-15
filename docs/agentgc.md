# 与 AgentGC 联合使用

CpyRef 保存正文；AgentGC 的程序 `agcc` 跟踪所选目录的文件变化。先确认现有跟踪，已有 live wrapper 覆盖项目和存储时复用它，不要叠加 watcher：

```sh
agcc --version
agcc --json sessions
```

没有覆盖这些目录的现有 wrapper 时，可从项目目录运行仓库中的[完整示例](../examples/agentgc-handoff.sh)：

```sh
bash /path/to/cpyref/examples/agentgc-handoff.sh HANDOFF.md patch=change.patch
```

`HANDOFF.md` 是交接说明文件，后续 `NAME=FILE` 是可选附件；这个示例不从 stdin 接收说明，`NOTE_FILE` 不能是 `-`。脚本需要 Bash、Python 3、Git、agcc，以及 CpyRef 0.4.0+。先用 `cpyref --version` 确认版本；可用 `CPYREF_BIN=/absolute/path/to/cpyref` 选择已安装或自行构建的程序。

脚本先只读发现精确 `store_path`，再用 `agcc run` 跟踪已有项目目录。默认存储基目录是项目内的 `.cpyref-local`，首次创建也发生在跟踪开始之后。Git 项目必须事先忽略 `.cpyref-local/`，且不能已经跟踪其中的文件；否则脚本停止，不会替你修改 Git 设置。

也可明确选择项目外**已存在的专用小目录**作为父目录，脚本会同时跟踪它和项目；不会向上寻找主目录并扩大扫描：

```sh
CPYREF_WORKFLOW_STORE_PARENT=/existing/private/cpyref-work \
  bash /path/to/cpyref/examples/agentgc-handoff.sh HANDOFF.md
```

实际基目录是该父目录下的 `.cpyref-local`。不要把主目录、文件系统根目录、共享资料目录或所有应用的数据目录作为父目录。CpyRef 的存储权限检查仍然生效；示例不改变权限、不清理数据。

创建成功后，脚本通过完整 token 的 `receive --list` 校验保存记录及交接哈希，并丢弃目录输出；再把可以直接交给下一 Agent 的短提示写到 stdout。它不会把说明或附件正文贴回终端。`verify` 用于核对当前来源文件，不能替代保存记录校验。

`agcc run` 自行完成跟踪，无需 `watch-stop`。子进程把实际 session 和 state-home 以一行 JSON 交给父进程；父进程在 wrapper 退出后查询该 session，检查状态、gaps、未解决事件和自动处理结果。stdout 只留下交接提示；诊断和实际编号写到 stderr。命令失败保留原退出码；交接成功但跟踪需要复查时退出 3。零计数仍不能证明所有中间事件都被记录。

需要展开跟踪证据时，使用 stderr 返回的实际 `SESSION_ID`；若使用了自定义 AgentGC state-home，给每条命令加相同的全局 `--home PATH`：

```sh
agcc diagnostics --session SESSION_ID
agcc list --session SESSION_ID --sort time --limit 20
agcc history --session SESSION_ID --limit 20
```

`list` 可能包含 baseline 已有文件；`history` 才能区分观察与变化。保存引用属于需要保留的交接资料，不能因位于临时目录或增长超过阈值就删除。完整分类与磁盘检查按 AgentGC 的调用约定执行。

创建、校验或最终跟踪失败时，先检查 stderr 留下的创建收据、实际引用和 session。命令可能已经写入存储，不能直接重跑整段；跟踪缺口也不能用任务结束后的扫描补写成完整历史。
