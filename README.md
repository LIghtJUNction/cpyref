# CpyRef

**Copy once. Paste exactly.**

为 AI Agent 保存原文、按短编号取回的 Rust 命令行工具。支持文件、片段、管道输入和测试日志；默认按项目隔离，支持同机多 Agent 协作。可用于 Linux、macOS 和 Windows，不依赖系统剪贴板。

一次性本地复制优先用 `cp`。需要复用、交接或保留日志时再用 cpyref；使用引用不保证减少 Token。

## 安装

```sh
cargo install cpyref --locked
cpyref --version
```

源码安装：`cargo install --git https://github.com/LIghtJUNction/cpyref --locked`。
无 Rust 环境的安装包与 AUR 用法见[安装与发布](docs/releases.md)。
Agent 可安装 [skills/cpyref](skills/cpyref/SKILL.md)，按需读取子命令的 `--help`。已有引用交接要求 0.3.0+；编号管道和编辑计划要求 0.3.1+；一步交接、有限读取、正文定位和存储路径发现要求 0.4.0+。

## 一步交接给下一位 Agent

**写交接内容 → 附加关键文件 → 转交取回命令。**

```sh
cpyref handoff --input HANDOFF.md --attach patch=change.patch
```

交接内容保存为 `notes`，附件按名称保存。默认输出可原样转交的短提示和完整取回命令，不展开正文；接收方先看目录，再按需少量读取或精确导出。只有交接内容时省略 `--attach`。生成的是历史快照，后续修改原文件不会更新它。

这是**同机引用**：下一位 Agent 必须能访问原存储和相应项目目录；跨机需要另行传输材料。发送方的一步创建及短提示、接收方的 `--list` / `--read` 均要求 0.4.0+。参数、stdin 输入、已有引用与失败恢复见[交接流程](docs/workflows.md#一步创建交接)。与 agentgc 联合使用时，先发现存储位置，再跟踪选定目录，见[可复制接入流程](docs/agentgc.md)。

用户说“用 cpy 交接”时，Agent 应调用实际程序 `cpyref`。想在 POSIX 交互终端少打几个字，可选用 `alias cpy=cpyref`；脚本仍写 `cpyref`，无需安装全局别名。

## 复制与粘贴

```sh
cpyref context
# 管道只传编号；正文不会经过模型。
cpyref copy input.txt --lines 20:40 | cpyref paste - --output snippet.rs

# 文件、唯一文本、管道输入。
cpyref copy input.txt
cpyref copy config.txt --text 'timeout = 30'
cat file.txt | cpyref copy
```

普通 `copy` 成功时，stdout 只有编号与换行，stderr 为空；编号为 `c` 加 12 个小写十六进制字符。失败时退出码非零，stdout 为空，stderr 为一行 JSON。`copy --json` 的编号在 `reference.id`。

`paste` 输出原始字节，不补换行。写文件优先用 `--output`；已有文件受保护，只有获准覆盖时才加 `--force`。管道须检查上游命令是否成功；Bash 可用 `set -o pipefail`。

| 选择方式 | 示例 |
| --- | --- |
| 行号，包含两端 | `--lines 20:40` |
| 唯一文本 | `--text 'timeout = 30'` |
| 正则与捕获组 | `--regex 'href="(?P<url>[^"]+)"' --group url` |
| 起止标记 | `--from BEGIN --to END --exclude-markers` |
| 缩小范围或消除歧义 | `--within 100:200`、`--before server_a`、`--after enabled` |

四类主选择器不能混用。多个匹配会报 `AMBIGUOUS`，应缩小范围；不要自动选择第一个。完整规则、预览、错误码见[命令参考](docs/cli-reference.md)。

## 只看需要的内容

```sh
cpyref read REF --lines 20:40 --max-bytes 2048
cpyref read REF --tail --max-bytes 1024
cpyref find REF 'error' --ignore-case --max-bytes 2048
cpyref list --kind run --ids
cpyref search 'timeout' --ids
```

`REF` 用实际完整编号替换。`read` 默认最多读取 4096 个输入字节；续读时保留 `--lines`，将返回的 `next_offset` 传给 `--offset`，内容始终限于所选行。`find` 在内存中扫描完整保存正文（最多 128 MiB），只返回匹配位置和有限上下文。两者不依赖原文件。`--ids` 每行只输出一个编号，便于继续交给工具处理。

以上 `read`、`find`、`--ids`、`list --kind` 和 `logs --tail` 要求 0.4.0+。预算按输入字节计算，不是模型 Token 数；JSON 转义与有损解码可能扩大输出。精确落盘仍用 `paste --output`，不要从预览重建原文。参数与分页规则见[有限读取与正文定位](docs/cli-reference.md#有限读取与正文定位)。

## 少传编号，不抄哈希

以下流程从 0.3.1 起提供：

```sh
set -o pipefail  # Bash；保留上游失败状态
cpyref copy source.rs --lines 10:20 | cpyref insert - target.rs --at-end --plan
# 检查返回的目标、位置和大小，在相同空间运行 next_command。
cpyref apply PLAN_ID
```

`PLAN_ID` 用实际返回的 `plan` 替换。第一条命令不修改目标；第二条自动核对计划中的来源和目标哈希。目标已变化就停止，不会自动刷新哈希。替换用 `cpyref replace ... --plan`。原有显式哈希方式保留；完整规则见[管道与编辑计划](docs/easy-edits.md)。

## 空间与安全

空间取决于**命令所在项目**，不是输入文件的位置。向上寻找最近的 `.git` 或 `.cpyref.toml`；同项目子目录共用空间，无标记时按当前目录隔离。

`--project /absolute/project` 固定项目；`--namespace team` 指定组。跨项目共享须明确使用 `--scope shared --namespace team`，并访问同一存储基目录。`--store` 不会自动取消隔离，也不提供远程同步。配置与多 Agent 用法见[项目空间](docs/project-spaces.md)。

快照独立于原文件；`paste` 在输出前检查长度和 SHA-256，`info` 只读元数据。整文件与按行复制保留二进制字节，文本匹配要求所选搜索范围为 UTF-8。默认输入上限 16 MiB，`--max-bytes` 的硬上限为 128 MiB，按整个输入计算。

数据存于本机，未加密；编号不是凭证，项目隔离不阻止同账号程序访问。没有自动过期或总磁盘配额。保存条数可设 `max_entries`；`clean` 默认只预览，删除需要 `--apply --confirm`。

## 按需使用

| 任务 | 命令或参考 |
| --- | --- |
| 看元数据、找引用 | `info`、`list --ids`、`search --ids`；[命令参考](docs/cli-reference.md) |
| 少量读取、定位正文 | `read`、`find`；[有限读取与正文定位](docs/cli-reference.md#有限读取与正文定位) |
| 统计、清理 | `stats`、`remove`、`clean`；[命令参考](docs/cli-reference.md) |
| 插入或替换 | `cpyref insert/replace ... --plan`、`cpyref apply PLAN_ID`；[编辑计划](docs/easy-edits.md)；[底层写入](docs/hardening.md) |
| 保存测试日志、核对来源 | `run`、`logs`、`snapshot`、`verify`；[工作流](docs/workflows.md) |
| 交接引用或命名包 | `handoff`、`receive`、`bundle`；[工作流](docs/workflows.md) |
| Agent 的调用与失败处理 | [调用约定](docs/agent-contract.md) |
| 复制粘贴评估与已有结果 | [CopyPaste Benchmark](benchmarks/copypaste/README.md) |

## 开发与许可证

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

贡献规则见 [AGENTS.md](AGENTS.md)。许可证为 [AUL 1.0-draft.1](LICENSE) © 2026 LIghtJUNction，自定义草案，尚未获 OSI 批准。[UPSTREAM.toml](UPSTREAM.toml) 指定上游；此前 MIT 版本及单独授权的第三方材料保留原条款，原 MIT 文本见 [LICENSE.MIT](LICENSE.MIT)。
