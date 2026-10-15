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
Agent 可安装 [skills/cpyref](skills/cpyref/SKILL.md)，按需读取子命令的 `--help`。下述交接和编辑功能要求 0.3.0+。

## 复制与粘贴

```sh
cpyref context
# 复制成功后，将实际编号直接传给 paste。
ref=$(cpyref copy input.txt --lines 20:40) &&
    cpyref paste "$ref" --output snippet.rs

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

## 空间与安全

空间取决于**命令所在项目**，不是输入文件的位置。向上寻找最近的 `.git` 或 `.cpyref.toml`；同项目子目录共用空间，无标记时按当前目录隔离。

`--project /absolute/project` 固定项目；`--namespace team` 指定组。跨项目共享须明确使用 `--scope shared --namespace team`，并访问同一存储基目录。`--store` 不会自动取消隔离，也不提供远程同步。配置与多 Agent 用法见[项目空间](docs/project-spaces.md)。

快照独立于原文件；`paste` 在输出前检查长度和 SHA-256，`info` 只读元数据。整文件与按行复制保留二进制字节，文本匹配要求所选搜索范围为 UTF-8。默认输入上限 16 MiB，`--max-bytes` 的硬上限为 128 MiB，按整个输入计算。

数据存于本机，未加密；编号不是凭证，项目隔离不阻止同账号程序访问。没有自动过期或总磁盘配额。保存条数可设 `max_entries`；`clean` 默认只预览，删除需要 `--apply --confirm`。

## 按需使用

| 任务 | 命令或参考 |
| --- | --- |
| 查看、查找、清理 | `info`、`list`、`search`、`stats`、`remove`、`clean`；[命令参考](docs/cli-reference.md) |
| 插入或替换 | `insert`、`replace`；先预览，再提供目标哈希；[安全写入](docs/hardening.md) |
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
