# 命令参考

安装与常用操作见 [README](../README.md)，空间配置见[项目隔离与多 Agent 协作](project-spaces.md)。

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
| 4 | `INVALID_ARGUMENT`、`INVALID_REGEX`、`INVALID_GROUP`、`INVALID_UTF8`、`INVALID_REF`、`UNSUPPORTED_SOURCE`、`CONFIG_ERROR`、`PROJECT_ERROR`、`PLAN_CHANGED` | 参数、匹配、编码、编号、输入类型、配置或项目路径错误 |
| 5 | `FILE_NOT_FOUND`、`PERMISSION_DENIED`、`OUTPUT_EXISTS`、`UNSAFE_STORE`、`UNSAFE_OUTPUT`、`IO_ERROR` 等 | 文件或存储操作失败 |
| 6 | `REF_NOT_FOUND` | 当前空间没有该编号；details.context 说明本次选择的空间 |
| 7 | `CORRUPT_REF` | 快照格式、长度或正文摘要不符 |
| 8 | `EMPTY_SELECTION` | 输入或所选内容为空 |
| 9 | `SIZE_LIMIT`、`MATCH_LIMIT`、`ENTRY_LIMIT` | 超过输入、元数据或匹配数量限制 |

歧义错误最多返回 20 个候选的位置，不返回候选正文；`match_count` 为实际候选数，`truncated` 表示位置列表是否截断。达到匹配上限时直接报 `MATCH_LIMIT`，不把截断后的结果当成唯一结果。

字节位置从 0 开始，范围为 `[byte_start, byte_end)`，结束位置不包含在内容中。行号从 1 开始。创建时间为 Unix 秒。给 Agent 的具体调用约定见 [Agent 调用约定](agent-contract.md)。

## 存储、限制与安全边界

存储基目录默认为当前用户本地数据目录下的 `cpyref`。project、directory、shared 范围在基目录内划分空间；只有显式 global 范围直接使用基目录中的旧版快照。路径结构与迁移说明见[项目隔离与多 Agent 协作](project-spaces.md)。

选区保存为独立的原始字节快照，不是指向原文件的动态指针。复制成功后，即使原文件被修改或删除，粘贴仍读取保存的内容。整个文件复制和行号复制不要求 UTF-8；文本匹配要求所搜索范围内的内容为 UTF-8。空格、缩进、CRLF、NUL 和非 ASCII 字符不会被重新生成或规范化。

每份快照使用单个文件保存元数据和正文。先写临时文件并同步文件内容，再以不覆盖已有编号的方式发布。编号冲突时重新生成，最多重试 32 次。正常多进程使用由工具自身的每空间写锁协调，不需要模型手工协调。粘贴在输出任何内容前检查完整正文的长度和 SHA-256。`info` 只读元数据，不验证正文摘要。这些检查不是来源认证，不防止有写权限的进程同时修改正文与元数据。

默认输入上限为 16 MiB，可通过 `--max-bytes` 调整，硬上限为 128 MiB。即使只复制其中一行，输入限制仍按整个读取的输入计算。匹配扫描最多接受 10,000 个结果；正则编译大小上限为 4 MiB。输入在内存中处理，不是无限文件的流式选择器，也不承诺整个进程内存仅等于输入大小。

Unix 下新建存储目录权限为 `0700`，快照为 `0600`；会检查基目录到实际空间的传统权限位并拒绝受管路径中的符号链接。Windows 使用用户目录及继承权限，并拒绝受管目录中的重解析点。这里不验证所有扩展 ACL，不阻止同账号程序主动访问其他空间或并发篡改路径，不改变已有目录权限。

数据未加密，短编号不是授权令牌。敏感任务应使用独立账号或受控存储，并通过 `remove` 删除不再需要的快照。没有自动过期、总磁盘配额或自动清理；可配置保存条数上限，并显式预览/执行批量清理。快照发布中断可能留下临时文件；这里不承诺突然断电后的目录项持久性。

复制期间应避免其他程序同时改写输入文件。当前版本保存读取到的字节，不对外部写入者加锁，也不声称获得文件系统级的一致性快照。

粘贴到文件默认禁止覆盖；`--force` 使用新文件替换已有普通文件，不保留原文件的权限、扩展属性或其他元数据。符号链接输出被拒绝。输出到 stdout 发生写入错误时，接收方可能已经收到了部分内容；复制编号输出失败时，已保存的快照也可能仍然存在。

## 备注、查找与复制预览

```sh
cpyref copy src/example.py --from 'def example' --to 'def next' \
  --exclude-end --note '修改前的函数'
cpyref list --limit 20
cpyref list --source 'src/' --note '修改前' --limit 20 --offset 20
cpyref list --json

# 只报告匹配数、位置和字节数；不写快照，也不创建存储目录
cpyref copy src/example.py --from 'def example' --to 'def next' --exclude-end --dry-run
# 显式查看少量内容；JSON 转义输出，不把控制字符直接送到终端
cpyref copy src/example.py --lines 10:20 --dry-run --show-content --preview-bytes 256
```

两端默认都包含；`--exclude-start` 和 `--exclude-end` 可单独使用，也可一起使用。
`--exclude-markers` 仍表示同时排除两端，不能与独立边界选项混用。这些选项只改变标记本身是否被包含，不额外删除换行或缩进。

`--dry-run` 成功返回 JSON：`match_count`、最多 20 个 `candidates`、`truncated`、实际 `selected` 和 `bytes`。
未指定 occurrence 且存在多个匹配时，仍返回退出码 3 的 `AMBIGUOUS`，在 stderr 提供最多 20 个位置及对应字节数，绝不默认选择第一处。
预览正文必须显式使用 `--show-content`；默认最多 256 个输入字节，`--preview-bytes` 范围 1–4096。正文显示使用 UTF-8 有损解码并进行 JSON 转义，可能截断多字节字符；此显示不是字节导出，精确取回请用 `paste`。`content_bytes` 是截取的输入字节数，`content_truncated` 表示是否截断。

备注最多 4096 个 UTF-8 字节，作为可选元数据保存，旧快照无需迁移。默认 copy 输出仍只有短编号。
`list` 默认输出元数据表，包含编号、来源、行范围、半开字节范围、字节数、Unix 秒时间和备注；来源与备注使用 JSON 字符串转义。默认最多 20 条，`--limit` 范围 1–1000，`--offset` 从 0 开始。按创建时间从新到旧排列，同秒按 ID 升序。`--source` 与 `--note` 是区分大小写的子串匹配，同时使用时取交集。
`--json` 返回 `references`、筛选后的 `total`、`offset`、`limit` 和 `context`。不读取正文；损坏的元数据会报错，不会悄悄漏掉引用。

## 统计与清理

```sh
cpyref stats
cpyref clean c0123456789ab c123456789abc
cpyref clean --note '临时'
cpyref clean --source 'build/' --older-than 30 --keep-latest 20
cpyref clean --all
# 检查预览后，重复相同选择条件，并填入预览的 confirm 值
cpyref clean --note '临时' --apply --confirm PREVIEW_TOKEN
```

这些命令始终只操作当前选定空间：默认是当前项目和 namespace，显式选择 shared/global 时才访问相应空间，不递归清理其他项目。

`stats` 返回引用数 `count`、正文总字节数 `payload_bytes`、快照文件总逻辑字节数 `stored_bytes`（含头部，不含锁文件、临时文件和文件系统块开销）、最大快照元数据 `largest`、最早/最新创建时间及年龄分布。分桶为不足 1 天、1 到不足 7 天、7 到不足 30 天、30 天及以上。空库的时间及 largest 为 null。只读元数据，不校验正文摘要，整个统计不保证在其他进程并发修改时为原子快照。

`clean` 默认只预览。必须给出完整编号、筛选条件、`--keep-latest` 或显式 `--all`，如果配置了 `[cleanup]` 默认规则，也可直接运行 `clean` 预览，否则空命令拒绝执行。编号不能与过滤条件或 `--all` 混用。空 source/note 筛选值被拒绝，不能借它们隐式全选。
`--older-than N` 选择创建时间早于 N 天的引用，每天固定 86400 秒；时间相等不选。`--keep-latest N` 保护整个当前空间中最新的 N 条，再与其他条件取交集；同秒按 ID 排序，不承诺真实创建先后。
配置表 `[cleanup]` 可设置默认 `older_than_days` 与 `keep_latest`；主动调用 clean 时使用，命令行覆盖对应字段。显式编号或 `--all` 不继承配置默认规则。没有任何定时、隐式年龄清理或达到上限后的自动淘汰。

预览返回完整候选元数据、数量、正文总字节数及 `confirm`。执行需同时传 `--apply --confirm TOKEN`，确认码绑定空间路径和候选元数据；集合变化返回 `PLAN_CHANGED`（退出码 4），要求重新预览。确认码是防止误删的检查值，不是授权凭证。相同账号的其他程序仍可能主动修改存储。
执行时先在写锁内重新检查全部候选元数据，再逐条删除；文件系统错误可能导致部分成功，错误详情会列出 `removed` 与 `failed`。不删除预览后新增的引用，不清理未知文件或遗留临时文件。元数据损坏时批量清理拒绝继续，可核对后用 `remove REF` 精确删除该记录。

保存条数通过 `max_entries`、`CPYREF_MAX_ENTRIES` 或 `--max-entries` 配置，见[配置规范](project-spaces.md#保存上限与配置规范)。超过上限返回 `ENTRY_LIMIT`（退出码 9）；不会自动删除任何引用。正常写入和删除使用每空间 `.write.lock` 文件协调，进程退出会释放操作系统锁；不要手工删锁文件。使用旧版本或直接操作存储的程序不受新版本条数上限约束。


## 测试记录、源码清单、来源核对与交接

`run`、`logs`、`snapshot`、`verify`、`handoff` 的参数、返回协议和边界见[工作流参考](workflows.md)。
`run` 正常保存后使用子命令的退出状态，失败命令仍在 stdout 返回已保存的测试摘要；执行器或记录错误仍使用 stderr JSON。
`verify`、`snapshot check/diff`、`logs --check` 的“不匹配/无法确认”返回退出码 10 并输出状态 JSON，这与 copy 的失败协议分开。
新错误包括 `WRONG_KIND`、`PATH_ENCODING`、`UNSAFE_SOURCE`、`SOURCE_CHANGED`、`REF_IN_USE`（退出码 4）、`RECORDING_FAILED`（退出码 5）和 `SNAPSHOT_CHANGED`（退出码 10）。
