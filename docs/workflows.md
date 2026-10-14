# 测试日志、源码清单与交接

这些命令与 copy/paste 共用当前项目、scope、namespace 和存储基目录。默认输出一行 JSON 摘要，不把完整日志或源码展开到终端。下面的 `REF`、`SOURCE_REF`、`RUN_REF` 必须替换成实际返回的完整短引用。

## 记录一次测试

```sh
cpyref run --input src --input tests --input Cargo.toml --input Cargo.lock \
  --artifact target/debug/cpyref -- cargo test --locked --all-targets
```

`--` 后是程序与参数，直接启动，不经过 shell。管道、重定向和 shell 内建命令需要明确调用对应 shell。子进程使用调用时的工作目录及继承环境；默认关闭 stdin，交互命令可显式加 `--inherit-stdin`。

记录包含参数列表、工作目录、开始/结束 Unix 秒时间、单调时钟耗时、原始退出码或终止信号，以及完整 stdout/stderr。两路日志分别通过管道流式写入私有临时文件，最终打包成一个不可变引用；不尝试恢复两路输出之间的精确先后顺序。

一次运行只占一个保存名额。`--input` 可重复，记录明确选中的文件哈希；未指定时 `source_after.status = "not_recorded"`，不会偷偷扫描整个仓库。`--artifact` 可重复，记录命令结束后声明输出的大小和哈希。文件当时存在不证明本次命令重建了它，`artifact_relationship` 会明确说明这一限制。环境变量不存入记录；命令参数和日志原样保存，可能包含你传入的敏感值。

默认摘要包含：

- `reference`：测试记录引用。
- `command_exit` / `signal`：命令的真实退出状态，不能由测试计数替代。
- `wrapper_exit`：cpyref 返回的退出码。正常保存后保留 0–255 范围的原码；Unix 信号映射为 `128 + signal`。Windows 等平台超过范围的退出码压到 255，完整值仍在 command_exit。
- `recording_status`、`logs_complete`：记录是否保存、日志是否捕获到 EOF。
- `analysis.counts`：通过、失败、跳过；无法确认时为 null。
- `source_after`：选定源码在命令结束后的变化，默认最多列出 20 个路径，省略数单列。

日志保存失败时返回 `RECORDING_FAILED`，工具退出码为 5，不能把命令成功误报成记录成功。错误详情另列命令退出状态、捕获完整性及保留的本地日志路径；若已完成打包，也保留打包记录的恢复路径。不要在这种情况下自动重跑命令，它可能已经产生实际效果。

开始前检查条数上限，发布时在写锁内再次检查。其他进程在运行中耗尽名额，或删除关联快照，会使最终记录保存失败；完整捕获日志保留供恢复。进程被强制杀死、机器断电等可能留下临时文件而没有最终引用，不能将它们算作已完成记录。若后代进程继续持有输出管道，run 会等待 EOF。没有隐式超时或后台任务管理。

### 测试统计的边界

`--parser auto` 只在程序名为 cargo/cargo.exe 时选择 Cargo/libtest 文本解析器，其他程序使用 none。也可显式指定 `--parser cargo` 或 `--parser none`。

第一轮内置解析器支持 Rust libtest 常规文本的 `running N tests` 和 `test result: ...`。只有套件开始/完成数量、测试数量等一致时才给出正式 counts；异常退出且没有已报告失败、未完成套件、计数溢出、超过 64 KiB 的单行或不支持的测量结果，会标 unknown。`observed_counts` 是读到的局部统计，不表示全部完成。ignored 单列为 skipped，filtered out 不算执行过，也不算通过。统计只覆盖日志中已执行的套件，不推断未启动的目标或整个项目的测试计划。

摘要最多展示 5 个失败位置和约 4 KiB 的上下文，提供省略计数；完整失败内容始终保留在原日志。`logs --failed` 可以继续分页。失败命令没有可识别位置时展示有长度限制的日志末尾，并保持计数未知。输出内容使用 JSON 转义，不直接向终端发送原日志控制字符。

## 按需查看日志和关联信息

```sh
cpyref logs RUN_REF
cpyref logs RUN_REF --failed --limit 10 --offset 0 --max-bytes 4096
cpyref logs RUN_REF --stream stderr --offset 0 --max-bytes 4096
cpyref logs RUN_REF --stream stdout --output full-stdout.log
cpyref logs RUN_REF --view command
cpyref logs RUN_REF --view inputs --limit 20 --offset 0
cpyref logs RUN_REF --view artifacts
cpyref logs RUN_REF --check
```

不带展开参数时只输出摘要。`--failed` 的 offset 是识别到的失败位置序号，默认 limit=20；`--stream` 的 offset 是原始字节偏移，返回 next_offset、总字节数及 truncated。`--max-bytes` 默认 4096，范围 1–65536，限制上下文或原始预览输入字节数，JSON 转义后长度可能增加。正文展示使用 UTF-8 有损解码；查看二进制或精确文本请用 `--output` 导出完整单路日志。

导出保护已有文件；`--force` 只允许替换普通文件，拒绝符号链接。完整导出不能使用非零 offset。首次输出任何正文或创建导出文件前，先流式校验整条记录的长度和 SHA-256；坏记录报 CORRUPT_REF，不先输出半段内容。读取与校验大日志的时间随日志大小增加。

`--view command` 展示参数索引、工作目录及时间，参数可按 limit/offset 分页，总展示字节由 max-bytes 限制。它是诊断数据，不是可执行的 shell 字符串。`--view inputs` 展示测试前明确记录的源文件及哈希；`--view artifacts` 展示声明输出及观察到的哈希。两者都默认最多 20 条，并列出省略数量。

`logs --check` 用保存的源清单和制品哈希核对当前文件，报告修改、删除、新增或无法确认。它不会改写历史测试结果，也不证明构建来源。全匹配退出 0，变化、未知或没有记录输入退出 10；路径详情支持 limit/offset 分页。

run 记录允许日志超过 copy 的 128 MiB 上限，使用流式存取；没有自动截断或磁盘容量配额。保存期间需要临时空间，峰值可能约为日志总量的三倍。每份 run 描述信息最多 4 MiB；不满足限制时保留恢复日志并报错。`paste` 拒绝直接展开 run 容器，使用 logs 选择 stdout/stderr。

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

文件其他部分变化不会自动使片段失效；也不只凭行号或修改时间判定。所有状态都返回 `semantic_position_verified=false`：字节仍存在不证明仍在原来的函数、作用域或语义位置。本轮不提供自动插入、替换或 patch，也不改变 paste --force 的整文件输出行为。

## 一条命令交接引用

```sh
cpyref handoff REF
cpyref handoff REF --format sh
cpyref handoff REF --format powershell
```

默认 JSON 包括完整 ID、上下文、工作目录、argv、需要清除的继承环境项，以及 same_host 标记。sh / powershell 模式直接输出一条正确引用路径的检索命令：bytes 用 paste，snapshot 用 snapshot show，run 用 logs。命令固定 no-config、存储基目录、scope、namespace、项目根和保存上限，清除可能与显式参数冲突的 CPYREF_CONFIG / CPYREF_PROJECT；shell 模式还固定工作目录，并使用当前运行的 cpyref 可执行文件绝对路径，避免 PATH 上的旧版本接手；不会把备注或来源正文拼进命令。

这不是数据传输或远程同步。接收者必须在同机访问同一存储，并有兼容版本的 cpyref；不要仅因为 ID 相同就猜测另一台机器存在同一份内容。生成命令本身只读、不执行该命令、不修改配置。

## 保留与清理

bytes、snapshot、run 共用短编号、list、stats、max_entries 和 clean。info 新增 kind（旧 bytes 记录可省略）及 dependencies。run 的两路日志与描述在同一个文件内，不会出现删掉日志却保留空测试摘要的情况。

关联独立 source snapshot 的 run 会声明依赖。remove/clean 不允许删除仍被保留记录依赖的快照，预览就报 REF_IN_USE；可以显式将依赖者和快照一起选入同一批次，工具按依赖顺序删除，不自动级联扩大范围。执行前在写锁内再次检查依赖，避免预览后新记录出现造成悬空引用。批量删除仍可能因文件系统错误部分成功，removed 字段列出已删项。

元数据损坏会阻止 list 和批量清理；可按完整编号逐条 remove 损坏记录进行恢复。已损坏、无法读取的头部不能提供可靠依赖保护。依赖保护由新版执行，旧版本或直接操作存储的程序不能保证这一保护。原有字节快照不需要迁移，copy 默认短 ID、paste 不添换行、输出保护和显式共享边界保持不变。
