# CpyRef

**Copy once. Paste exactly.**

为 Agent 保存原文、按短编号准确粘贴的 Rust 命令行工具。默认按项目隔离剪贴板，支持明确配置的多 Agent 协作空间。

CpyRef 不要求模型重新生成长链接、代码片段或工具输出。它从文件或标准输入中读取内容，保存选定的原始字节，只返回短编号。后续用编号取出相同内容。

## 当前范围

实现 `copy`、`paste`、`info`、`remove` 和 `context`。面向 Linux、macOS 和 Windows；仓库 CI 在三类系统上执行测试与 Clippy 检查。这是本地引用式复制粘贴，不依赖桌面环境或系统剪贴板。

首版 CLI 已合并 PR #1。项目隔离功能位于 PR #2 的 `feat/project-isolation` 分支。尚未发布 crates.io 包；`Cargo.toml` 设置了 `publish = false`，避免意外发布。仓库尚未指定开源许可证。

## 安装

使用当前 stable Rust 与 Cargo。首次构建需要访问 crates.io。

```sh
# PR #2 合并前，安装包含项目隔离功能的分支
cargo install --git https://github.com/LIghtJUNction/cpyref --branch feat/project-isolation

# 或在该分支的仓库目录内安装
cargo install --path .

cpyref --help
cpyref copy --help
cpyref context
```

## 基本用法

```sh
# 复制整个文件
cpyref copy input.txt

# 复制第 20 到 40 行，包括第 40 行的原始换行符
cpyref copy src/main.rs --lines 20:40

# 只复制一个唯一出现的文本片段
cpyref copy config.txt --text 'timeout = 30'

# 读取工具输出；这里由工具传输内容，不让模型重新输出原文
some-command | cpyref copy -
```

成功时，默认 stdout 只有一行短编号，例如：

```text
c8f12a093be47d
```

编号格式为 `c` 加 12 个小写十六进制字符，共 13 个字符。上面只是格式示例，请使用命令实际返回的编号。编号只在所选空间中有效，不是访问凭证，也不是内容摘要。

```sh
# 在同一项目和 namespace 中输出原始字节，不额外添加换行
cpyref paste c8f12a093be47d

# 写入文件；默认拒绝覆盖已有文件
cpyref paste c8f12a093be47d --output snippet.rs

# 明确允许替换已有普通文件
cpyref paste c8f12a093be47d --output snippet.rs --force

# 仅查看来源、字节数、行范围、摘要、创建时间，不展开正文
cpyref info c8f12a093be47d

# 删除这个空间中的快照；也可使用 rm 子命令
cpyref remove c8f12a093be47d
```

直接写文件时，优先使用 `--output`。不要假定所有 shell 的管道和重定向都能透明传递二进制内容，尤其是不同版本的 PowerShell。

## 项目隔离与协作

默认从当前工作目录向上找到最近的 `.git` 文件/目录或 `.cpyref.toml`，将其所在目录作为项目根。同一项目的子目录共用空间，不同项目分开。如果没有项目标记，则按当前目录隔离。无 Git 的项目可以创建一个空的 `.cpyref.toml` 来标记根目录。

**空间由运行目录决定，不由输入文件所在目录决定。** 两个项目即使文件夹同名、Git 远程地址相同或 namespace 同名，默认也不共用数据。Git 子模块和独立工作目录默认分开。

```sh
# 查看生效项目、范围、组名和实际存储路径；不会创建目录或展开正文
cpyref context

# 同项目内部按 Agent 分组，或让多个 Agent 选择同一个组协作
cpyref --namespace agent-a copy input.txt
cpyref --namespace review-team copy input.txt

# 需要每个文件夹独立，而不是整个项目共用
cpyref --scope directory copy input.txt

# 明确跨项目共享一个有名字的空间
cpyref --scope shared --namespace team-build copy input.txt
cpyref --scope shared --namespace team-build paste c8f12a093be47d

# 固定一个项目，不因 Agent 切换工作目录而改变空间
cpyref --project /work/project-a copy input.txt
```

四种范围为 `project`（默认）、`directory`、`shared` 和 `global`。shared 必须明确指定 namespace。`--project` 只适用于 project 范围，且不会改变输入和输出文件的相对路径。跨项目或选错组后查询编号，`paste`、`info` 和 `remove` 都返回 `REF_NOT_FOUND`，不会自动搜索其他空间。

支持 `CPYREF_SCOPE`、`CPYREF_NAMESPACE`、`CPYREF_PROJECT`、`CPYREF_STORE` 和 TOML 配置。参数优先于环境变量；自动项目配置优先于用户配置。使用 `--config FILE` 可明确选择一个可信配置文件并替代全部自动配置；`--no-config` 跳过文件设置，但仍识别项目标记。

项目内的 `.cpyref.toml` 可以写：

```toml
scope = "project"
namespace = "review-team"
```

自动项目配置不能启用 shared/global 或重定向 store，以免仓库附带的文件悄悄改变共享范围。跨项目共享使用明确参数、环境变量或显式选择的可信配置。配置示例见 [examples/project.toml](examples/project.toml) 与 [examples/shared.toml](examples/shared.toml)。

**兼容性变化：`--store` 和 `CPYREF_STORE` 现在选择存储基目录，默认仍按项目划分子目录。** 旧版快照不删除、不自动迁移，也不混入新项目空间；明确加上 `--scope global` 可以访问原来的平铺存储。

完整的路径规则、配置优先级、协作示例和旧数据访问方式见 [docs/project-spaces.md](docs/project-spaces.md)。这是避免误混的空间划分，不是阻止同系统账号恶意进程的权限系统，也没有新增远程同步。

## 复杂匹配

### 正则与分组

```sh
# 复制唯一匹配的完整 URL
cpyref copy response.txt --regex 'https://[^\s"]+'

# 只复制命名分组中的 URL，不包含 href= 和引号
cpyref copy page.html --regex 'href="(?P<url>[^"]+)"' --group url

# 跨行正则：. 可以匹配换行
cpyref copy response.txt --regex 'BEGIN.*?END' --dotall

# ^ 和 $ 匹配行边界
cpyref copy config.txt --regex '^timeout\s*=.*$' --multiline

# 不区分大小写地定位，但保存原始大小写
cpyref copy notes.txt --text 'important' --ignore-case
```

使用 Rust `regex` 的语法。正则匹配为从左到右、不重叠的匹配；不支持反向引用和前后查找断言。需要前后条件时可用下面的上下文过滤。不存在的捕获分组会报错。可选分组未参与匹配时，该结果不产生候选片段。零长度匹配不会生成成功编号。

### 起止标记

```sh
# 默认包含标记本身
cpyref copy report.txt --from 'BEGIN RESULT' --to 'END RESULT'

# 只复制标记之间的原始内容
cpyref copy report.txt --from 'BEGIN RESULT' --to 'END RESULT' --exclude-markers

# 起止标记也可使用正则
cpyref copy report.txt --from '^BEGIN [0-9]+$' --to '^END$' --markers-regex --multiline
```

配对规则是明确的：每个起点与其后最近、且不与起点重叠的终点配对。多个完整片段仍会返回 `AMBIGUOUS`。这不是嵌套语法解析，不能用它保证提取完整函数、嵌套 XML 或任意括号结构。

### 范围与上下文

```sh
# 仅在文件的第 100 到 200 行内搜索
cpyref copy source.rs --text 'return result;' --within 100:200

# 要求匹配之前有 server_a，之后有 enabled
cpyref copy config.txt --text 'timeout = 30' --before 'server_a' --after 'enabled' --context-lines 3

# 明确选择过滤后的第 2 个匹配
cpyref copy config.txt --text 'timeout = 30' --occurrence 2
```

`--before` 和 `--after` 是精确、区分大小写的文本过滤。它们不会被 `--ignore-case` 改变。上下文窗口包括匹配所在行的剩余部分，以及指定数量的前后行。`--context-lines 0` 只看同一行。上下文不越过 `--within` 范围。

处理顺序为：限制范围 → 匹配 → 上下文过滤 → 检查唯一性或选择指定次数。`--occurrence` 从 1 开始，不能作为隐藏的默认策略。

四类主选择器 `--lines`、`--text`、`--regex`、`--from/--to` 不能混用。`--within` 可与其组合。行号始终是原文件中的绝对行号，从 1 开始，首尾均包含。文件末尾的换行符不会产生额外的虚构空行。

精确文本检查重叠匹配。例如，`banana` 中的 `ana` 有两个候选，而不是一个。

## 成功与失败协议

默认复制成功：退出码 `0`，stdout 只有 `ID\n`，stderr 为空。

`cpyref copy input.txt --json` 成功时输出 `ok`、`reference` 和 `context`。`reference.id` 是短编号；reference 还包含版本、字节数、完整 SHA-256、来源、创建时间和选区。context 与 `cpyref context` 的输出一致。两者都不展开正文。

复制失败：stdout 为空，stderr 是一行 JSON，退出码非零。例如无法唯一定位时：

```json
{"ok":false,"error":{"code":"AMBIGUOUS","message":"Multiple matches found; narrow the selector or specify --occurrence.","details":{"match_count":2,"candidates":[{"occurrence":1,"span":{"byte_start":0,"byte_end":4,"line_start":1,"line_end":1,"selector":"text"}},{"occurrence":2,"span":{"byte_start":5,"byte_end":9,"line_start":2,"line_end":2,"selector":"text"}}],"truncated":false}}}
```

| 退出码 | 主要错误代码 | 含义 |
| --- | --- | --- |
| 0 | 无 | 成功，包括帮助、版本和 context 查询 |
| 2 | `NO_MATCH` | 无法定位、行不存在、指定次数不存在、缺少起止边界 |
| 3 | `AMBIGUOUS` | 多个匹配，不能唯一定位 |
| 4 | `INVALID_ARGUMENT`、`INVALID_REGEX`、`INVALID_GROUP`、`INVALID_UTF8`、`INVALID_REF`、`UNSUPPORTED_SOURCE`、`CONFIG_ERROR`、`PROJECT_ERROR` | 参数、匹配、编码、编号、输入类型、配置或项目路径错误 |
| 5 | `FILE_NOT_FOUND`、`PERMISSION_DENIED`、`OUTPUT_EXISTS`、`UNSAFE_STORE`、`UNSAFE_OUTPUT`、`IO_ERROR` 等 | 文件或存储操作失败 |
| 6 | `REF_NOT_FOUND` | 当前空间没有该编号；details.context 说明本次选择的空间 |
| 7 | `CORRUPT_REF` | 快照格式、长度或正文摘要不符 |
| 8 | `EMPTY_SELECTION` | 输入或所选内容为空 |
| 9 | `SIZE_LIMIT`、`MATCH_LIMIT` | 超过输入、元数据或匹配数量限制 |

歧义错误最多返回 20 个候选的位置，不返回候选正文；`match_count` 为实际候选数，`truncated` 表示位置列表是否截断。达到匹配上限时直接报 `MATCH_LIMIT`，不把截断后的结果当成唯一结果。

字节位置从 0 开始，范围为 `[byte_start, byte_end)`，结束位置不包含在内容中。行号从 1 开始。创建时间为 Unix 秒。给 Agent 的具体调用约定见 [docs/agent-contract.md](docs/agent-contract.md)。

## 存储、限制与安全边界

存储基目录默认为当前用户本地数据目录下的 `cpyref`。project、directory、shared 范围在基目录内划分空间；只有显式 global 范围直接使用基目录中的旧版快照。路径结构与迁移说明见项目隔离文档。

选区保存为独立的原始字节快照，不是指向原文件的动态指针。复制成功后，即使原文件被修改或删除，粘贴仍读取保存的内容。整个文件复制和行号复制不要求 UTF-8；文本匹配要求所搜索范围内的内容为 UTF-8。空格、缩进、CRLF、NUL 和非 ASCII 字符不会被重新生成或规范化。

每份快照使用单个文件保存元数据和正文。先写临时文件并同步文件内容，再以不覆盖已有编号的方式发布。编号冲突时重新生成，最多重试 32 次。正常多进程使用不需要模型协调锁。粘贴在输出任何内容前检查完整正文的长度和 SHA-256。`info` 只读元数据，不验证正文摘要。这些检查不是来源认证，不防止有写权限的进程同时修改正文与元数据。

默认输入上限为 16 MiB，可通过 `--max-bytes` 调整，硬上限为 128 MiB。即使只复制其中一行，输入限制仍按整个读取的输入计算。匹配扫描最多接受 10,000 个结果；正则编译大小上限为 4 MiB。输入在内存中处理，不是无限文件的流式选择器，也不承诺整个进程内存仅等于输入大小。

Unix 下新建存储目录权限为 `0700`，快照为 `0600`；会检查基目录到实际空间的传统权限位并拒绝受管路径中的符号链接。Windows 使用用户目录及继承权限，并拒绝受管目录中的重解析点。这里不验证所有扩展 ACL，不阻止同账号程序主动访问其他空间或并发篡改路径，不改变已有目录权限。

数据未加密，短编号不是授权令牌。敏感任务应使用独立账号或受控存储，并通过 `remove` 删除不再需要的快照。目前没有自动过期、总磁盘配额或自动清理。快照发布中断可能留下临时文件；这里不承诺突然断电后的目录项持久性。

复制期间应避免其他程序同时改写输入文件。当前版本保存读取到的字节，不对外部写入者加锁，也不声称获得文件系统级的一致性快照。

粘贴到文件默认禁止覆盖；`--force` 使用新文件替换已有普通文件，不保留原文件的权限、扩展属性或其他元数据。符号链接输出被拒绝。输出到 stdout 发生写入错误时，接收方可能已经收到了部分内容；复制编号输出失败时，已保存的快照也可能仍然存在。

## 开发

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

测试覆盖：原始字节、CRLF、Unicode、重叠匹配、正则分组、标记、上下文、行号、失败输出、大小上限、快照损坏、并发进程、原文件修改或删除、覆盖保护、项目边界、namespace、配置优先级、旧全局空间、跨项目协作和路径检查。

当前没有系统剪贴板接入、MCP 服务、函数或类的语法结构匹配、模糊匹配、远程共享、自动过期、内容去重或就地插入/替换源文件的功能。不要把这些当作已实现能力。
