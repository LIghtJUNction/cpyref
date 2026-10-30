<div align="center">
  <h1>CpyRef</h1>
  <p><strong>Copy once. Paste exactly.</strong></p>
  <p>为 AI Agent 保存原始字节，再用短编号精确取回、交接或生成受校验的编辑计划。</p>

  [![Crates.io](https://img.shields.io/crates/v/cpyref?style=flat-square&logo=rust)](https://crates.io/crates/cpyref)
  [![Release](https://img.shields.io/github/v/release/LIghtJUNction/cpyref?style=flat-square)](https://github.com/LIghtJUNction/cpyref/releases/latest)
  [![Rust](https://img.shields.io/badge/Rust-1.89%2B-111111?style=flat-square&logo=rust)](Cargo.toml)
  [![License](https://img.shields.io/badge/license-AUL%201.0--draft.1-111111?style=flat-square)](LICENSE)

  [安装](#安装) · [一分钟理解](#一分钟理解) · [Agent 交接](#一步交接给下一位-agent) · [有限读取](#只取需要的内容) · [编辑计划](#带哈希保护的编辑计划)
</div>

## 一分钟理解

CpyRef 本质上是一个**按内容保存的本地引用仓库**：

```text
文件 / 片段 / stdin / 测试日志
               ↓
       保存原始字节 + SHA-256
               ↓
           c4f8a0c7b19e
               ↓
 read / paste / handoff / verify / edit plan
```

Agent 后续只传短编号，不必反复把大段正文抄进对话：

```bash
cpyref copy input.txt --lines 20:40
# stdout: c4f8a0c7b19e

cpyref paste c4f8a0c7b19e --output snippet.rs
```

`paste` 输出保存时的原始字节，不自动补换行。写出前会核对长度与 SHA-256。

它不是系统剪贴板，也不是远程对象存储。一次性本机复制优先使用 `cp`；需要复用、分页查看、跨 Agent 交接或保留历史证据时再用 CpyRef。使用引用不保证减少模型 Token，实际效果取决于宿主如何把工具输出放进上下文。

## 适合什么任务

| 场景 | CpyRef 提供的最小机制 |
| --- | --- |
| 大文件只取一个片段 | 先保存，再按行、文本、正则或标记精确选择 |
| Agent 交接 | 一次保存说明与命名附件，转交取回命令 |
| 测试日志留证 | 保存运行输出，之后按尾部或关键字有限读取 |
| 自动编辑 | 先生成计划，再核对源/目标哈希后应用 |
| 多 Agent 同机协作 | 按项目与 namespace 隔离，共享时显式授权 |
| 历史内容核验 | 快照独立于原文件，可重新计算并验证哈希 |

## 安装

### crates.io

```bash
cargo install cpyref --locked
cpyref --version
```

### 源码

```bash
cargo install --git https://github.com/LIghtJUNction/cpyref --locked
```

### 预编译包

[GitHub Releases](https://github.com/LIghtJUNction/cpyref/releases/latest) 提供 Linux、macOS 与 Windows 产物，并包含部分 `.deb`、`.rpm` 与 Arch 包格式。AUR 和发布校验见 [安装与发布](docs/releases.md)。

当前主线为 `0.4.0`。一步交接、有限读取、正文定位和存储路径发现要求 0.4.0+；编号管道与编辑计划要求 0.3.1+。

Agent 可以安装配套 [CpyRef skill](skills/cpyref/SKILL.md)，但仍应以当前二进制的 `--help` 作为参数事实来源。

## 复制与精确取回

```bash
# 当前项目空间与存储位置
cpyref context

# 整文件
cpyref copy input.txt

# 包含两端的行范围
cpyref copy input.txt --lines 20:40

# 唯一文本
cpyref copy config.txt --text 'timeout = 30'

# stdin
cat build.log | cpyref copy
```

普通 `copy` 成功时，stdout 只有 `c` 加 12 位小写十六进制编号和换行，stderr 为空。失败时退出码非零、stdout 为空、stderr 为单行 JSON，方便 Agent 不靠模糊文本猜状态。

可用选择器：

| 选择器 | 示例 |
| --- | --- |
| 行号 | `--lines 20:40` |
| 唯一文本 | `--text 'timeout = 30'` |
| 正则捕获 | `--regex 'href="(?P<url>[^"]+)"' --group url` |
| 起止标记 | `--from BEGIN --to END --exclude-markers` |
| 缩小歧义 | `--within 100:200`、`--before server_a`、`--after enabled` |

四类主选择器不能混用。多个匹配返回 `AMBIGUOUS`，不会偷偷选择第一个。完整规则和错误码见 [CLI 参考](docs/cli-reference.md)。

## 一步交接给下一位 Agent

```bash
cpyref handoff --input HANDOFF.md --attach patch=change.patch
```

这条命令会：

1. 把交接说明保存为 `notes`；
2. 把附件按名称保存；
3. 输出可原样转发的短提示和完整取回命令。

接收方先看目录，再按需有限读取或精确导出。快照在创建时固定，原文件后续变化不会修改历史交接。

```text
发送方机器上的 CpyRef 存储
            ↓ 同机可访问
       下一位 Agent
```

这是同机交接，不包含远程同步。跨机器必须另外传输存储材料或实际附件。详细失败恢复、stdin、已有引用和 bundle 流程见 [交接工作流](docs/workflows.md#一步创建交接)。

用户说“用 cpy 交接”时，实际程序仍是 `cpyref`。交互终端可选 `alias cpy=cpyref`，脚本应写完整命令。

## 只取需要的内容

```bash
cpyref read REF --lines 20:40 --max-bytes 2048
cpyref read REF --tail --max-bytes 1024
cpyref find REF 'error' --ignore-case --max-bytes 2048
cpyref list --kind run --ids
cpyref search 'timeout' --ids
```

`read` 默认最多返回 4096 个输入字节。续读时保留原选择范围，并把返回的 `next_offset` 传给 `--offset`。`find` 可以在内存中扫描完整保存内容，但只返回有限上下文和匹配位置。

预算按输入字节计算，不是模型 Token。JSON 转义和有损文本显示可能扩大输出；需要精确字节时使用 `paste --output`，不要从预览反向拼回文件。

## 带哈希保护的编辑计划

CpyRef 可以把“引用内容写到目标位置”拆成预览与应用两步：

```bash
set -o pipefail
cpyref copy source.rs --lines 10:20 \
  | cpyref insert - target.rs --at-end --plan

cpyref apply PLAN_ID
```

第一步不修改目标，只记录：

```text
来源引用 + 来源哈希
目标文件 + 目标哈希
插入/替换位置
预期结果
```

第二步重新核对源与目标。目标在计划后发生变化就停止，不自动刷新哈希，也不把旧计划强行应用到新文件。替换使用 `cpyref replace ... --plan`。

这本质上是一个很小的乐观并发控制：

```pseudo
if current_source_hash != planned_source_hash: abort
if current_target_hash != planned_target_hash: abort
apply_exact_change()
```

完整命令见 [编辑计划](docs/easy-edits.md)；底层安全写入见 [hardening](docs/hardening.md)。

## 项目空间与协作边界

CpyRef 根据**命令当前所在项目**决定空间，而不是根据输入文件的位置：

```text
从当前目录向上寻找最近的 .git 或 .cpyref.toml
                 ↓
              项目空间
```

同项目子目录默认共享；没有标记时按当前目录隔离。

```bash
cpyref --project /absolute/project context
cpyref --namespace team list --ids
cpyref --scope shared --namespace team copy input.txt
```

跨项目共享必须显式使用 `--scope shared --namespace ...`，并让参与者访问同一存储基目录。`--store` 只改变存放位置，不自动取消项目隔离，也不提供网络同步。

详见 [项目空间](docs/project-spaces.md)。

## 数据与安全

- 快照独立于原文件；原文件删除或修改不影响已保存内容。
- 整文件和按行复制保留二进制字节；文本匹配要求搜索范围是 UTF-8。
- 默认输入上限 16 MiB；可配置硬上限为 128 MiB。
- 数据保存在本机，默认不加密。
- 短编号是定位符，不是访问凭证。
- 项目隔离不能阻止同一操作系统账号下的其他程序读取存储。
- 默认没有自动过期和总磁盘配额；可设置 `max_entries`。
- `clean` 默认只预览；实际删除要求 `--apply --confirm`。
- 写入已有文件默认拒绝，只有明确获准时才使用 `--force`。

## 命令地图

| 目标 | 命令 / 文档 |
| --- | --- |
| 查引用与元数据 | `info`、`list --ids`、`search --ids`；[CLI 参考](docs/cli-reference.md) |
| 有限读取与搜索 | `read`、`find`；[正文定位](docs/cli-reference.md#有限读取与正文定位) |
| 保存运行证据 | `run`、`logs`、`snapshot`、`verify`；[工作流](docs/workflows.md) |
| 交接与命名包 | `handoff`、`receive`、`bundle`；[工作流](docs/workflows.md) |
| 编辑目标文件 | `insert/replace --plan`、`apply`；[编辑计划](docs/easy-edits.md) |
| 统计与清理 | `stats`、`remove`、`clean`；[CLI 参考](docs/cli-reference.md) |
| Agent 错误处理 | [Agent 调用合同](docs/agent-contract.md) |
| 与 agentgc 联用 | [存储发现与跟踪](docs/agentgc.md) |
| 效果评估 | [CopyPaste Benchmark](benchmarks/copypaste/README.md) |

## 开发验证

```bash
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

源码禁止 `unsafe`。贡献规则见 [AGENTS.md](AGENTS.md)。

## License

当前原创材料使用 [AUL 1.0-draft.1](LICENSE)，这是尚未获 OSI 批准的自定义草案。[UPSTREAM.toml](UPSTREAM.toml) 指定上游反馈位置；此前 MIT 版本及单独授权的第三方材料继续保留原条款，原 MIT 文本见 [LICENSE.MIT](LICENSE.MIT)。
