# CpyRef

**Copy once. Paste exactly.**

为 AI Agent 保存原文、按短编号取回的 Rust 命令行工具。文件、代码片段或工具输出只需复制一次，后续通过编号粘贴原始字节，避免模型重新生成内容。

默认按项目隔离，支持同机多 Agent 协作。支持 Linux、macOS 和 Windows，不依赖系统剪贴板或桌面环境。

## 安装

需要 stable Rust 与 Cargo，首次构建需要访问 crates.io。从 GitHub 源码安装：

```sh
cargo install --git https://github.com/LIghtJUNction/cpyref
cpyref --help
```

也可以克隆仓库后运行 `cargo install --path .`。

## 基本用法

```sh
# 复制整个文件
cpyref copy input.txt

# 复制第 20 到 40 行，保留原始换行符
cpyref copy src/main.rs --lines 20:40

# 复制唯一出现的文本片段
cpyref copy config.txt --text 'timeout = 30'

# 直接保存工具输出
some-command | cpyref copy -
```

复制成功时，stdout 只有一行短编号：

```text
c8f12a093be47d
```

编号格式为 `c` 加 12 个小写十六进制字符。下面使用的是示例编号，实际操作请换成 `copy` 返回的编号，并保持同一项目和 namespace（分组名）。

```sh
# 输出原始字节，不额外添加换行
cpyref paste c8f12a093be47d

# 写入文件，默认拒绝覆盖已有文件
cpyref paste c8f12a093be47d --output snippet.rs

# 明确允许替换已有普通文件
cpyref paste c8f12a093be47d --output snippet.rs --force

# 查看来源、大小和摘要，不展开正文
cpyref info c8f12a093be47d

# 删除快照，也可使用 rm 子命令
cpyref remove c8f12a093be47d

# 查看当前项目、分组和存储路径
cpyref context
```

直接写文件时优先使用 `--output`，避免 shell 管道或重定向改变二进制内容。

## 选择内容

| 需求 | 参数示例 |
| --- | --- |
| 按行复制 | `--lines 20:40` |
| 精确文本 | `--text 'timeout = 30'` |
| 正则匹配 | `--regex 'https://[^\s"]+'` |
| 正则捕获分组 | `--regex 'href="(?P<url>[^"]+)"' --group url` |
| 起止标记之间的内容 | `--from 'BEGIN' --to 'END' --exclude-markers` |
| 限制搜索行范围 | `--text 'timeout = 30' --within 100:200` |
| 按上下文筛选 | `--text 'timeout = 30' --before 'server_a' --after 'enabled'` |

行号从 1 开始，范围包含两端。多个匹配会报 `AMBIGUOUS`；可缩小范围、增加上下文，或明确使用 `--occurrence N`。行、文本、正则和起止标记这四类主选择器不能混用。

更多选项与匹配规则见[命令参考](docs/cli-reference.md)，也可以运行 `cpyref copy --help`。

## 项目隔离与协作

空间由**运行命令时所在的项目**决定，不由输入文件的位置决定。工具向上寻找最近的 `.git` 或 `.cpyref.toml`：同项目子目录共用空间，不同项目分开；没有标记时按当前目录隔离。

```sh
# 同项目的多个 Agent 使用同一个分组
cpyref --namespace review-team copy input.txt
cpyref --namespace review-team paste c8f12a093be47d

# 固定项目，避免切换工作目录后改变空间
cpyref --project /work/project-a copy input.txt

# 明确跨项目共享；各次调用需使用相同参数
cpyref --scope shared --namespace team-build copy input.txt
cpyref --scope shared --namespace team-build paste c8f12a093be47d
```

跨项目协作要求同机访问同一存储基目录，不提供远程同步。分组、配置、环境变量和旧版快照访问见[项目隔离与多 Agent 协作](docs/project-spaces.md)。

## 输出与限制

- 默认复制成功：退出码 `0`，stdout 为 `ID\n`，stderr 为空。`copy --json` 返回 `reference.id`、元数据和空间信息，不展开正文；失败时 stdout 为空，stderr 为一行 JSON，退出码非零。
- 保存的是独立快照。原文件修改或删除后仍可粘贴；空格、缩进、CRLF、NUL 和非 ASCII 字符保持原样。整文件与按行复制支持二进制，文本匹配要求搜索范围为 UTF-8。
- 默认输入上限为 16 MiB，可用 `--max-bytes` 调整，硬上限为 128 MiB。限制按整个输入计算；输入在内存中处理。
- `paste` 在输出前校验正文长度与 SHA-256；`info` 只读取元数据。完整错误代码和存储细节见[命令参考](docs/cli-reference.md)。
- 数据存于用户本地数据目录下的 `cpyref`，未加密。短编号不是凭证，项目隔离不阻止同账号程序访问。快照没有自动过期或总容量配额，可用 `remove` 删除。

Agent 的调用顺序、失败处理和内容来源约定见 [Agent 调用约定](docs/agent-contract.md)。

## 开发

```sh
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

## 许可证

[MIT](LICENSE) © 2026 LIghtJUNction
