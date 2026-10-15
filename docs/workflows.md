# 测试日志、源码清单与交接

这些命令与 copy/paste 共用当前项目、scope、namespace 和存储基目录。日志与核对命令默认输出一行 JSON 摘要，新建交接默认输出短提示，不把完整日志或源码展开到终端。下面的 `REF`、`SOURCE_REF`、`RUN_REF` 必须替换成实际返回的完整短引用。

## 一步创建交接

本节的一步创建、短提示和接收端有界查看要求 CpyRef 0.4.0+。

```sh
cpyref handoff --input HANDOFF.md --attach patch=change.patch --attach config=example.toml
# Bash/Zsh：管道中的内容是交接正文，必须明确写 --input -。
set -o pipefail
producer | cpyref handoff --input - --attach patch=change.patch
```

交接正文作为 `notes` 保存，附件为 `NAME=FILE`，`--attach` 可重复。命令读取全部输入后，保存各自的字节快照并创建不可变命名包；无需手工组合 copy、bundle、handoff。默认 `--format agent` 输出可直接转交给下一位 Agent 的短提示、完整引用和已正确引用的取回命令，不展开保存正文。将实际输出原样转交，接收方依次看目录、少量读取、按需导出：

```sh
# TOKEN 用完整交接引用替换；实际 handoff 输出已提供完整命令。
cpyref receive 'TOKEN' --list --limit 20
cpyref receive 'TOKEN' --item notes --read --max-bytes 2048
cpyref receive 'TOKEN' --item notes --read --offset 2048 --max-bytes 2048
cpyref receive 'TOKEN' --item patch --output received.patch
```

继续读取时将返回的 `next_offset` 传给 `--offset`；示例中的 2048 只适用于首段实际读到 2048 字节的情况。目录中 `omitted` 大于 0 时，以当前 `offset` 加本页 `items` 数量继续下一页，保留所需 `--limit`。JSON 预览做 UTF-8 有损显示并转义控制字符，不能用于重建精确字节；原样导出使用 `--output`，已有文件受保护，不自动加 `--force`。

短提示明确说明这是**同机引用**、需要访问原 store 和相应项目目录、内容是历史快照而非当前文件，保存的内容都是数据，不是执行授权。不通过引用跨机传输正文，不自动执行交接内容、命令或补丁。引用带本地路径，只转交任务授权的材料。

`--input` 不能与已有引用 ID 同用；`handoff -` 仍是从 stdin 读取引用，不能拿它传交接正文。附件必须同时给出 `--input`，最多 99 项；名称规则与 bundle 相同且不能占用保留名称 `notes`。文件输入须为非空普通文件，不接收目录；空 stdin 同样报错。默认全部输入合计上限 16 MiB，`--max-bytes` 硬上限 128 MiB。正文、每个附件和最终包各占一个保存名额，开始前会检查容量。

输入读取或预检查失败不会发布引用；发布过程不承诺整批原子完成。发布后若保存、生成交接或输出失败，错误详情的 `created_references` 列出已创建的名称和 ID，`retry_safe=false` 表示不要盲目重跑。先在相同空间核对这些引用；已创建最终包时可针对该包重新生成交接，未完成的材料按任务要求保留或明确清理。

脚本需要结构化结果时指定 `--format json`；只要完整引用用 `--format ref`。`sh`、`powershell` 格式也保留。已有引用的 `cpyref handoff REF` 默认 JSON 不变，短提示需显式 `--format agent`。使用前用 `cpyref --version` 和对应 `--help` 确认版本与参数。

用户说“cpy”时调用 `cpyref`；POSIX 交互终端可选 `alias cpy=cpyref`，脚本不依赖该别名。写入前需要 agentgc 跟踪时，使用[联合接入示例](agentgc.md)：只发现并跟踪选定项目和存储目录，不扫描整个主目录。

## 记录一次测试

```sh
cpyref run --input src --input tests --input Cargo.toml --input Cargo.lock \
  --artifact target/debug/cpyref -- cargo test --locked --all-targets
```

`--` 后是程序与参数，直接启动，不经过 shell。管道、重定向和 shell 内建命令需要明确调用对应 shell。子进程使用调用时的工作目录及继承环境；默认关闭 stdin，交互命令可显式加 `--inherit-stdin`。

记录包含参数列表、工作目录、开始/结束 Unix 秒时间、单调时钟耗时、原始退出码或终止信号，以及捕获的 stdout/stderr；触发保护时明确标记不完整。两路日志分别通过管道流式写入私有临时文件，最终打包成一个不可变引用；不尝试恢复两路输出之间的精确先后顺序。

一次运行只占一个保存名额。`--input` 可重复，记录明确选中的文件哈希；未指定时 `source_after.status = "not_recorded"`，不会偷偷扫描整个仓库。`--artifact` 可重复，记录命令结束后声明输出的大小和哈希。文件当时存在不证明本次命令重建了它，`artifact_relationship` 会明确说明这一限制。环境变量不存入记录；命令参数和日志原样保存，可能包含你传入的敏感值。

默认摘要包含：

- `reference`：测试记录引用。
- `command_exit` / `signal`：命令的真实退出状态，不能由测试计数替代。
- `wrapper_exit`：cpyref 返回的退出码。正常保存后保留 0–255 范围的原码；Unix 信号映射为 `128 + signal`。Windows 等平台超过范围的退出码压到 255，完整值仍在 command_exit。
- `recording_status`、`logs_complete`：记录是否保存、日志是否捕获到 EOF。
- `analysis.counts`：通过、失败、跳过；无法确认时为 null。
- `source_after`：选定源码在命令结束后的变化，默认最多列出 20 个路径，省略数单列。

日志保存失败时返回 `RECORDING_FAILED`，工具退出码为 5，不能把命令成功误报成记录成功。错误详情另列命令退出状态、捕获完整性及保留的本地日志路径；若已完成打包，也保留打包记录的恢复路径。不要在这种情况下自动重跑命令，它可能已经产生实际效果。

开始前检查条数上限，发布时在写锁内再次检查。其他进程在运行中耗尽名额，或删除关联快照，会使最终记录保存失败；完整捕获日志保留供恢复。进程被强制杀死、机器断电等可能留下临时文件而没有最终引用，不能将它们算作已完成记录。默认 600 秒超时包含命令退出后的管道排空；后台后代继续持有输出管道也受此限制。触发保护时结束本次进程组（Windows Job），取消捕获，清理最多等待 2 秒；异常清理失败报 RECORDING_FAILED 并保留恢复文件。主动脱离进程组的进程不保证被终止，run 不是进程沙箱。

### 测试统计的边界

`--parser auto` 只在程序名为 cargo/cargo.exe 时选择 Cargo/libtest 文本解析器，其他程序使用 none。也可显式指定 `--parser cargo` 或 `--parser none`。

第一轮内置解析器支持 Rust libtest 常规文本的 `running N tests` 和 `test result: ...`。只有套件开始/完成数量、测试数量等一致时才给出正式 counts；异常退出且没有已报告失败、未完成套件、计数溢出、超过 64 KiB 的单行或不支持的测量结果，会标 unknown。`observed_counts` 是读到的局部统计，不表示全部完成。ignored 单列为 skipped，filtered out 不算执行过，也不算通过。统计只覆盖日志中已执行的套件，不推断未启动的目标或整个项目的测试计划。

摘要最多展示 5 个失败位置和约 4 KiB 的上下文，提供省略计数；完整捕获的失败内容保留在原日志；触发日志上限或超时时，未捕获部分不可恢复，logs_complete=false。`logs --failed` 可以继续分页。失败命令没有可识别位置时展示有长度限制的日志末尾，并保持计数未知。输出内容使用 JSON 转义，不直接向终端发送原日志控制字符。

## 执行和空间保护

```sh
cpyref run --max-log-bytes 67108864 --timeout 600 --min-free-bytes 536870912 -- cargo test
```

`--max-log-bytes` 范围 1–1073741824，`--timeout` 为秒、范围 1–86400；不支持 0 表示无限。`--min-free-bytes` 为保留空闲字节数，可显式设 0，但仍检查打包所需空间。启动前要求空闲空间至少为保留量 + 三倍日志上限 + 描述信息空间，否则 DISK_SPACE，命令不启动。执行中每约 250 ms 检查保留量及已捕获日志的打包余量，空间不足时停止命令并尝试保存部分记录。

环境变量为 `CPYREF_RUN_MAX_LOG_BYTES`、`CPYREF_RUN_TIMEOUT_SECONDS`、`CPYREF_RUN_MIN_FREE_BYTES`。TOML 配置：

```toml
[run]
max_log_bytes = 67108864
timeout_seconds = 600
min_free_bytes = 536870912
```

优先级为命令行 > 环境变量 > 自动项目配置 > 用户配置 > 默认值；显式 `--config` 替代自动文件。自动项目 run 设置仅作用于 project/directory 范围。`context` 显示配置解析后的默认限制，run 摘要显示本次最终限制。旧记录的 limits/termination_reason 为 null，不能据此假定历史执行有保护。

这是日志捕获限制和空闲空间检查，不是整个存储的总配额，也不限制命令自行写入的文件；其他程序同时写磁盘仍可能使最终保存失败。超时覆盖子进程执行与管道捕获，源码扫描、哈希、同步和最终打包仍需 I/O 时间。无需且不会自动删除旧引用来腾空间。

## 按需查看日志和关联信息

```sh
cpyref logs RUN_REF
cpyref logs RUN_REF --failed --limit 10 --offset 0 --max-bytes 4096
cpyref logs RUN_REF --stream stderr --offset 0 --max-bytes 4096
cpyref logs RUN_REF --stream stderr --tail --max-bytes 2048
cpyref logs RUN_REF --stream stdout --output full-stdout.log
cpyref logs RUN_REF --view command
cpyref logs RUN_REF --view inputs --limit 20 --offset 0
cpyref logs RUN_REF --view artifacts
cpyref logs RUN_REF --check
```

不带展开参数时只输出摘要。`--failed` 的 offset 是识别到的失败位置序号，默认 limit=20；`--stream` 的 offset 是原始字节偏移，返回 next_offset、总字节数及 truncated。`--max-bytes` 默认 4096，范围 1–65536，限制上下文或原始预览输入字节数，JSON 转义后长度可能增加。正文展示使用 UTF-8 有损解码；查看二进制或精确文本请用 `--output` 导出完整单路日志。

`--stream stdout|stderr --tail` 要求 0.4.0+，直接读取所选日志末尾最多 max-bytes 个字节，默认 4096。返回的 offset/next_offset 仍是该路日志的绝对字节位置；读到末尾时 truncated=false，不表示此前没有省略。`--tail` 必须与 `--stream` 同用，不能指定 `--offset`、`--failed`、`--check`、`--view` 或 `--output`。

导出保护已有文件；`--force` 只允许替换普通文件，拒绝符号链接。完整导出不能使用非零 offset。首次输出任何正文或创建导出文件前，先流式校验整条记录的长度和 SHA-256；坏记录报 CORRUPT_REF，不先输出半段内容。读取与校验大日志的时间随日志大小增加。

`--view command` 展示参数索引、工作目录及时间，参数可按 limit/offset 分页，总展示字节由 max-bytes 限制。它是诊断数据，不是可执行的 shell 字符串。`--view inputs` 展示测试前明确记录的源文件及哈希；`--view artifacts` 展示声明输出及观察到的哈希。两者都默认最多 20 条，并列出省略数量。

`logs --check` 用保存的源清单和制品哈希核对当前文件，报告修改、删除、新增或无法确认。它不会改写历史测试结果，也不证明构建来源。全匹配退出 0，变化、未知或没有记录输入退出 10；路径详情支持 limit/offset 分页。

run 使用流式存取，默认 stdout/stderr 合计最多 64 MiB，可显式提高到 1 GiB。达到上限后仍有输出时停止命令，保留前缀，termination_reason=log_limit、logs_complete=false、wrapper_exit=125；超时使用 timeout 和 124，原始命令退出码/信号另列。所有保护触发后正式 counts=null，即使前缀包含完整测试摘要。保存期间峰值可能约为日志总量的三倍。每份 run 描述信息最多 4 MiB；不满足限制时保留恢复日志并报错。`paste` 拒绝直接展开 run 容器，使用 logs 选择 stdout/stderr。

## 保存和核对源文件清单

```sh
cpyref snapshot src tests Cargo.toml Cargo.lock
cpyref snapshot check SOURCE_REF
cpyref snapshot show SOURCE_REF --limit 20 --offset 0
cpyref snapshot diff SOURCE_REF_A SOURCE_REF_B
cpyref run --snapshot SOURCE_REF --artifact target/debug/cpyref -- cargo test --locked
```

快照保存的是**路径、字节数和 SHA-256 清单，不包含源码正文，也不是编译器实际依赖追踪**。`content_saved=false`、`dependency_trace=false` 会明确返回。需要备份原始字节时继续使用 copy。

路径相对于调用目录，必须处于当前 project 根内；其他 scope 以当前目录作为源根。目录递归展开，最多 10000 个文件，重叠选择去重。固定跳过 `.git` 和位于源根内的存储基目录，排除规则保存在清单里；其他文件不按 Git ignore 静默排除。建议明确选择 src、锁文件和构建配置，避免把整个 target 纳入。拒绝符号链接、重解析点、特殊文件和非 UTF-8 路径；读取失败或哈希时检测到变化会报错，不声称快照完整。

check 会重新扫描原来的选择范围，因此能发现目录中新增的文件。文件其他属性或修改时间变化不会单独判为失效，最终比较的是保存的长度和哈希。hash 时的元数据检查只用于识别明显并发写入，不提供文件系统冻结或对恶意同账号写入者的防护。

check/diff 全匹配退出 0，有变化退出 10，并在 stdout 给出 added、removed、modified 数量及路径。默认最多 20 条，支持 limit/offset；它们不会自动展开源码。两个不同源根的清单拒绝比较。

`run --snapshot` 在启动前要求清单与当前文件全匹配，否则 SNAPSHOT_CHANGED，命令不会开始。它不能和 `--input` 混用。结束后重新核对选定源码；运行中变化会记录 changed，读取失败会记录 unknown。开始/结束一致仍不能证明文件在中途完全没有被改过，也不能证明命令实际读取了所有声明输入。

## 核对普通复制片段的来源

```sh
cpyref verify REF
```

先验证保存的正文，再读取当前来源，比较原始字节偏移，必要时查找所有重叠的精确匹配。结果为：

| status | 含义 |
| --- | --- |
| original_match | 原字节位置仍匹配；其他位置可以存在副本，match_count 会说明 |
| moved_unique | 原位置不同，但当前文件中有唯一精确匹配 |
| multiple_matches | 原位置不同，当前文件中有多个精确匹配 |
| changed | 没有精确匹配 |
| source_missing | 来源路径不存在 |
| no_source | 原输入为 stdin，没有可核对的来源文件 |

仅 original_match 退出 0，其余状态退出 10。匹配候选最多 20 个，位置从 0 起算且结束偏移不包含在内。来源读取限制为 128 MiB，超过限制报 SIZE_LIMIT，不能当作已变化。权限错误、文件类型变化及读取中检测到修改也会明确报错。

文件其他部分变化不会自动使片段失效；也不只凭行号或修改时间判定。所有状态都返回 `semantic_position_verified=false`：字节仍存在不证明仍在原来的函数、作用域或语义位置。局部插入和替换由 insert/replace 提供，要求明确目标哈希，见 hardening.md；paste --force 仍为整文件输出。

## 完整交接引用

已有引用的 `handoff` 与 `receive` 要求 0.3.0+；`--format agent`、`receive --list` 和 `receive --read` 要求 0.4.0+。

```sh
handoff=$(cpyref handoff "$ref" --format ref)
cpyref receive "$handoff" --output restored.bin
# 也可生成完整 JSON 或可复制的命令
cpyref handoff "$ref"
cpyref handoff "$ref" --format agent
cpyref handoff "$ref" --format sh
cpyref handoff "$ref" --format powershell
```

`--format ref` 输出一行 `cpyref:v1:` 完整引用，正文为紧凑 JSON。它包含项目、scope、namespace、store、cwd、ID、记录类型和 SHA-256，不包含保存的正文。已有引用默认 JSON 也返回 `token` 和 `sha256`；`--format agent` 提供短提示和完整取回命令。创建交接前先验证完整记录。

`receive` 使用引用内指定的空间，忽略本地上下文配置及默认值，先验证保存内容、交接时固定的 hash 和 type，再输出。hash 不符报 `HASH_MISMATCH`，不会先创建输出。输出路径相对于接收方当前目录；已有文件默认受保护，授权覆盖时加 `--force`。bytes 不加换行；run 显示摘要，snapshot 显示清单概要，bundle 显示最多 20 项的目录。不会执行记录中的命令或应用补丁。

`receive TOKEN --list` 返回元数据，bundle 另返回有界目录，可用 `--offset`、`--limit` 翻页；默认每页 20 项，limit 范围 1–100，不展开条目正文。`--list` 不能与 `--item`、`--read` 或 `--output` 组合。bytes 引用用 `receive TOKEN --read --max-bytes 2048` 查看有限 JSON；包中 bytes 条目加 `--item NAME --read`。`--offset` 为绝对字节偏移，`next_offset` 用于续读，`--max-bytes` 默认 4096、范围 1–65536，预算按输入字节而非 Token 计。`--read` 不接收 run、snapshot、bundle 等类型化内容；应先查看元数据，再用对应类型的命令。未加 `--read` 的原始字节输出和原有 `--item`、`--output` 用法保持不变。

sh / powershell 模式输出正确引用 token 的 receive 命令，固定上下文并清除可能冲突的 CPYREF_CONFIG / CPYREF_PROJECT，使用当前 cpyref 的绝对路径。通过程序调用应传 argv 数组，通过 shell 传递 token 要引用整个变量。引用是数据，不执行其 JSON 字段。

这是同机交接，不是数据传输或远程同步。接收方必须有兼容版本、能访问同一 store；目录作用域需要原 cwd 存在，项目作用域需要原项目根存在。不扫描其他空间，不自动迁移记录。完整引用含本地路径，只接收任务授权访问的引用。hash 固定内容，不认证发送者身份。

## 批量交接包

```sh
patch=$(cpyref copy change.patch)
notes=$(cpyref copy handoff.md)
# tests 使用先前 cpyref run 返回的实际引用
package=$(cpyref bundle create "patch=$patch" "notes=$notes" "tests=$tests")
cpyref bundle show "$package"             # 只有目录，没有正文
cpyref bundle read "$package" notes       # 默认最多 4096 字节
cpyref bundle read "$package" notes --offset 4096 --max-bytes 4096
cpyref bundle read "$package" patch --output review.patch
cpyref bundle read "$package" tests       # 只显示该测试收据摘要

handoff=$(cpyref handoff "$package" --format ref)
cpyref receive "$handoff"                 # 接收方看目录
cpyref receive "$handoff" --item patch --output received.patch
```

包是不可变的命名清单，每项固定 ID、hash、type 和字节数，正文只存一份。每包 1–100 项，名称为 1–64 个 ASCII 字母、数字、`_`、`.`、`-`，以字母或数字开头，不得重复。创建时所有引用须在当前同一空间，并验证完整内容。默认只输出短 ID，`--json` 返回元数据和空间。

`bundle show --limit 20 --offset 0` 分页显示目录，不读取条目正文。`bundle read` 只验证、读取所选项：bytes 返回有 `truncated`、`next_offset` 的有界 JSON（UTF-8 有损显示），需要原始字节时使用 `--output`；已有输出文件受保护。run/snapshot/bundle 只显示摘要或目录，更多视图使用所选 ID 的 logs/snapshot/bundle 命令；类型化条目不能用 `--output` 导出普通文本。不存在的名称报 `ITEM_NOT_FOUND`。嵌套包按需展开一层，不递归展开全部正文。

包占一个保存名额，声明对每项的依赖。包仍存在时 remove/clean 拒绝删除其依赖；删除包不会级联删除各项。可以在同一次明确的清理预览中选择包和各项。

## 保留与清理

bytes、snapshot、run、bundle 共用短编号、list、stats、max_entries 和 clean。info 新增 kind（旧 bytes 记录可省略）及 dependencies。run 的两路日志与描述在同一个文件内，不会出现删掉日志却保留空测试摘要的情况。

关联独立 source snapshot 的 run 会声明依赖。remove/clean 不允许删除仍被保留记录依赖的快照，预览就报 REF_IN_USE；可以显式将依赖者和快照一起选入同一批次，工具按依赖顺序删除，不自动级联扩大范围。执行前在写锁内再次检查依赖，避免预览后新记录出现造成悬空引用。批量删除仍可能因文件系统错误部分成功，removed 字段列出已删项。

元数据损坏会阻止 list 和批量清理；可按完整编号逐条 remove 损坏记录进行恢复。已损坏、无法读取的头部不能提供可靠依赖保护。依赖保护由新版执行，旧版本或直接操作存储的程序不能保证这一保护。原有字节快照不需要迁移，copy 默认短 ID、paste 不添换行、输出保护和显式共享边界保持不变。

明确敏感值的写入前处理见 [hardening.md](hardening.md)：`run --redact-env NAME` 只处理明确指定值，处理后的日志不再声称是原始字节。
