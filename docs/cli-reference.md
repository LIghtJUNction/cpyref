# 命令参考

安装与常用操作见 [README](../README.md)，空间配置见[项目隔离与多 Agent 协作](project-spaces.md)。

## 交接入口与存储发现

```sh
cpyref handoff --input HANDOFF.md --attach patch=change.patch
cpyref context --store-path
```

第一条命令保存交接内容和关键文件，默认只输出可原样转交的短提示、完整引用和取回命令；接收方先看目录，再按需读取。已有引用的 `handoff REF` 默认仍为 JSON，可用 `--format agent` 获取短提示。完整参数、`receive --list` / `--read` 和失败恢复见[交接流程](workflows.md#一步创建交接)。

`context --store-path` 只输出当前项目、scope、namespace 和 store 配置解析后选定的实际存储目录，加一个换行；不创建存储，不展开 JSON，也不扫描该目录。路径必须可精确表示为 UTF-8，否则报错，不输出有损路径。命令行的全局空间参数仍适用。路径可能尚不存在；与 agentgc 联合跟踪见[接入流程](agentgc.md)。

实际程序名是 `cpyref`；“用 cpy 交接”应映射到它。POSIX 交互终端可选 `alias cpy=cpyref`，脚本保留完整程序名。一步创建、`--format agent`、`context --store-path` 和 `receive --list` / `--read` 要求 0.4.0+；已有引用的原有交接格式要求 0.3.0+。

## 管道输入

```sh
cat file.txt | cpyref copy
producer | cpyref copy --lines 2:10 --note 'piped selection'
```

省略来源时读取管道中的原始字节，与显式 `copy -` 相同，支持二进制、选区、备注及 dry-run。不加换行；空输入不生成引用。直接在终端运行无参数 `copy` 会立即报错，避免等待输入。需要交互 stdin 时显式使用 `copy -`。来源记录为 `<stdin>`，后续 verify 无法核对原文件。Bash/Zsh 需保留上游命令失败时，先设置 `set -o pipefail`。

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

每份快照使用单个文件保存元数据和正文。先写临时文件并同步文件内容，再以不覆盖已有编号的方式发布。编号冲突时重新生成，最多重试 32 次。正常多进程使用由工具自身的每空间写锁协调，不需要模型手工协调。`paste`、`read`、`find` 在输出任何正文前检查完整正文的长度和 SHA-256。`info` 只读元数据，不验证正文摘要。这些检查不是来源认证，不防止有写权限的进程同时修改正文与元数据。

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
cpyref list --kind run --ids

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
`list` 默认输出元数据表，包含编号、来源、行范围、半开字节范围、字节数、Unix 秒时间和备注；来源与备注使用 JSON 字符串转义。默认最多 20 条，`--limit` 范围 1–1000，`--offset` 从 0 开始。按创建时间从新到旧排列，同秒按 ID 升序。`--source` 与 `--note` 是区分大小写的子串匹配，`--kind bytes|run|snapshot|bundle|edit-plan` 限制类型，各筛选取交集。
`--json` 返回 `references`、筛选后的 `total`、`offset`、`limit` 和 `context`。不读取正文；损坏的元数据会报错，不会悄悄漏掉引用。
`list --kind` 和 `--ids` 要求 0.4.0+。`--ids` 只输出本页完整编号，每行一个，无表头；筛选、排序和分页不变，不能与 `--json` 同用。空页不输出内容。

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

## 搜索已保存引用

```sh
cpyref search '修改前' --json
cpyref search 'def example' --source 'src/' --limit 20 --offset 0
cpyref search 'TEST' --ignore-case --kind bytes
cpyref search 'function' --metadata-only
cpyref search 'timeout' --kind bytes --ids
```

query 是 1–4096 UTF-8 字节的字面子串，默认区分大小写。匹配 ID、来源、备注、kind，以及 bytes 引用的已保存正文；其他类型只搜元数据。查询始终限制在当前项目、scope 和 namespace，查询的是保存内容而非现在的源文件。默认只显示元数据和 matched_in，正文不展开。`--metadata-only` 跳过正文，损坏正文仍可按元数据找回编号；默认正文搜索边扫描边校验完整哈希，损坏时报 CORRUPT_REF。

`--source`、`--note` 为区分大小写的元数据子串筛选，`--kind` 的类型与 list 相同；各筛选取交集。排序、limit 和 offset 与 list 相同，JSON 返回 total 和本页 references。ID 子串只能用来发现，其他操作仍需要完整 ID。搜索流式读取正文，内存有界；总耗时随选定空间中正文总量增长。

`search --ids` 要求 0.4.0+，与 `list --ids` 相同，只改变输出格式，不能与 `--json` 同用。`search` 用于找引用；已知编号后，用 `find` 定位该引用的正文。

## 有限读取与正文定位

`read` 与 `find` 要求 CpyRef 0.4.0+。

```sh
cpyref read REF
cpyref read REF --offset 4096 --max-bytes 2048
cpyref read REF --tail --max-bytes 1024
cpyref read REF --lines 20:40 --max-bytes 2048
cpyref find REF 'error' --ignore-case --limit 10 --max-bytes 2048
cpyref copy report.txt | cpyref read - --max-bytes 512
```

`REF` 用实际完整编号替换。`read` 与 `find` 只接受 bytes 引用；其他类型报 `WRONG_KIND`，run 日志使用 `cpyref logs RUN_REF --stream stderr --tail`，见[日志查看](workflows.md#按需查看日志和关联信息)。两者始终输出 JSON，读取已保存正文，不要求原文件仍存在。即使只看少量内容，也先校验完整正文；损坏时报 `CORRUPT_REF`，不输出未校验的片段。

`read` 默认从字节 `0` 开始，最多取 `4096` 个输入字节。`--offset` 是保存正文中的绝对字节位置；`--max-bytes` 范围为 1–65536。`--tail` 取正文最后最多 max-bytes 个字节，不能与 `--offset` 或 `--lines` 同用。`--lines N` 或 `--lines START:END` 选择保存正文中的行，从 1 开始、包含两端及原换行，行号不是复制前的源文件行号。`--lines` 可与 `--offset` 组合；偏移始终按绝对字节解释，并限制在该行范围的起止位置内。

返回编号 `reference`、哈希 `sha256`，以及 `offset`、`next_offset`、`total_bytes`、`bytes`、`truncated`、`encoding` 和 `content`。`bytes` 是本次截取的输入字节数；`truncated` 表示所选范围还有未读内容。行选择另返回固定的绝对字节边界 `selection_start`、`selection_end`。继续读取用返回的 `next_offset` 作为 `--offset`；它始终是保存正文中的绝对字节位置，不是行号。保留同一 `--lines` 即可安全续读，例如 `cpyref read REF --lines 20:40 --offset NEXT --max-bytes 2048`，将 NEXT 替换为返回的偏移；读取不会越过该行范围，truncated=false 时结束。

`find` 搜索 1–4096 UTF-8 字节的字面子串，默认区分大小写，`-i` / `--ignore-case` 忽略大小写。匹配从左到右且互不重叠；与 `copy --text` 的重叠候选规则不同。`matches` 每项返回序号 `index`、`byte_start/end`、`line_start/end`，以及 `content_start/end` 和有限的 `content`；位置都相对于保存正文。`--context-bytes` 控制每次匹配两侧的上下文，默认 80，范围 0–4096；`--max-bytes` 默认 4096，是本页所有片段合计的输入字节预算，不是每项各有一份。预算耗尽仍保留本页所选匹配的位置，并用各项的 `context_truncated` 标出省略；顶层 `content_bytes` 为实际片段字节数，`content_truncated` 汇总本页省略状态。

`find --offset` 是从 0 开始的匹配序号，不是字节位置；`--limit` 限制本页返回项数，默认 20，范围 1–1000；`--max-bytes` 范围 1–65536，只限制片段字节数。find 在内存中读取、扫描完整保存正文，上限为 128 MiB，预算和分页不会减少扫描量。返回准确的匹配总数 `total`，用 `next_offset` 继续下一页，`truncated` 表示还有后续匹配，与片段预算是否耗尽分开。没有匹配时成功返回空列表。`read`、`find` 的 `REF` 均可用 `-` 从 stdin 接收一个完整编号或成功的 copy JSON；JSON 中的空间必须与当前调用一致。检查管道上游的退出状态，不把正文当编号传入。

正文使用 `encoding: "utf-8-lossy"` 并进行 JSON 转义，二进制或截断的多字节字符可能显示为替代字符。输入字节预算不等于模型 Token 预算，也不限制 JSON 封装后的总长度。预览不能用于精确复制；保留原始字节请用 `cpyref paste REF --output FILE`。
