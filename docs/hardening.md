# 安全写入与运行限制（0.2.0）

这些功能属于 0.2.0 源码。0.1.1 发布包不包含它们。所有示例中的 REF 和 HASH 都是占位符，必须使用实际结果。

## 局部插入和替换

```sh
cpyref copy snippet.rs
# 用实际编号替换 REF；预览不会写文件，也不展开正文。
cpyref insert REF target.rs --after-line 20 --dry-run
# 检查预览的目标、字节位置和大小，然后使用返回的 before_sha256。
cpyref insert REF target.rs --after-line 20 --expect-sha256 HASH

cpyref replace REF config.txt --text 'old value' --dry-run
cpyref replace REF config.txt --text 'old value' --expect-sha256 HASH
```

`insert` 必须选择一个位置：`--before-line N`、`--after-line N`、`--before-text TEXT`、`--after-text TEXT`、`--at-start` 或 `--at-end`。`replace` 必须使用 `--lines A:B` 或 `--text TEXT`。文本要求唯一匹配，多个匹配报 `AMBIGUOUS`，不会默认选择第一个。

行号从 1 开始，行范围包含两端和原始行终止符。`--after-line` 插在该行全部字节之后，包括现有换行符；`--after-text` 插在标记字节之后。工具不自动添加换行、调整缩进或解析函数结构。空目标可以用 `--at-start` 或 `--at-end` 插入。

实际写入必须提供完整 `--expect-sha256`。缺少时返回 `PRECONDITION_REQUIRED`。目标与预览不同，或写入前复查发现变化时，返回 `TARGET_CHANGED`（退出码 10）。不能自动刷新哈希后盲目重试，否则会丢失这项保护。损坏的引用、不支持的记录类型、无匹配、歧义和超出大小限制均不会改写目标。

写入先构造同目录临时文件，保留普通文件权限，完成写入及文件同步后才替换目标，不先清空旧文件。目标和输出各有 128 MiB 上限。不改动未选中字节。拒绝只读文件、符号链接路径和 Windows 重解析点；Unix 还拒绝硬链接及 set-ID 文件。

预览返回前后 SHA-256、修改的字节范围、插入/删除字节数和 `applied=false`。执行返回相同类型报告及 `applied=true`，不展开正文。

### 并发与文件属性边界

同一存储基目录下的编辑器以规范化目标路径加锁，跨项目和 namespace 的调用也共用该目标锁。锁文件保留在存储基目录的 `edit-locks` 中，不写进源码目录；不能在编辑期间删除锁文件。要求同一目标版本的并发写入至多一个成功，后来的调用看到变化后失败。

锁只约束使用这套协议和相同存储基目录的进程。外部编辑器、不同存储基目录、恶意同账号程序不在保证范围内。对不合作进程，最后一次检查与替换之间仍存在竞态窗口；这不是文件系统级比较交换，也不是安全隔离。

只保证正文和普通权限，不保证保留所有者、扩展属性、自定义 ACL、备用数据流或硬链接关系。不要用于依赖这些属性的文件。临时文件替换不能保证断电后的目录持久性；不提供自动撤销。

## 限制命令运行和日志增长

```sh
cpyref run --timeout 120 --max-log-bytes 8388608 -- cargo test --locked
```

默认 `--timeout 900`，单位为秒，覆盖启动后的命令运行和等待输出管道关闭。默认 `--max-log-bytes 67108864`，即 stdout/stderr 合计 64 MiB。两路输出共用上限，不是每路各一份额度。原始读取量和脱敏后写入量分别不能超过该额度。描述信息、源码清单和已有历史记录不包含在日志额度内。

明确设置某一限制为 `0` 才关闭它。例如 `--timeout 0` 只关闭超时，并不会同时关闭日志上限。关闭限制前要评估命令的行为和可用磁盘。日志记录的最终打包仍需要临时空间，峰值可能接近日志预算的三倍加描述信息；这不是整个存储的总磁盘配额。

到达限制后，工具尝试终止所创建的进程组：Unix 使用进程组，Windows 使用 Job Object。即使主进程已退出，但后代进程仍持有 stdout/stderr，也受同一超时约束。主动逃离进程组、无法中断的进程及操作系统故障不属于可靠终止保证；无法确认结束时会报错并保留恢复路径，不无限等待读取线程。

运行组只用于有结束点的命令，不应用于启动需在命令结束后继续驻留的服务。它不是恶意程序的资源沙箱，不能限制子进程自行写出的其他文件、内存或网络。

| 状态 | 工具退出码 | 记录字段 |
| --- | --- | --- |
| 正常结束 | 原命令退出码（沿用现有映射） | `stop_reason=null` |
| 超时停止 | 124 | `stop_reason="timeout"`，`logs_complete=false` |
| 日志超限停止 | 125 | `stop_reason="log_limit"`，`logs_complete=false` |
| 捕获或保存失败 | 5 | `RECORDING_FAILED`，附可用恢复路径 |

原始 `command_exit` 和信号仍单独记录。即使主命令先返回 0，也不能把随后因管道超时或日志超限停止的工作报告为成功。不完整记录的正式测试计数保持未知，已观察到的局部统计不代表完整结果。保存和解析阶段不在命令超时范围内。

## 在写入磁盘之前处理明确指定的敏感值

```sh
# API_TOKEN 已存在于当前环境中；命令参数只传变量名，不传密钥本身。
cpyref run --redact-env API_TOKEN --timeout 120 -- some-command
```

可以重复 `--redact-env NAME`。工具只读取明确指定的环境变量，用 `[REDACTED]` 替换 stdout/stderr 以及保存的参数列表中的完整原值。流式处理会保留必要的尾部，防止值分散在两次读取中时漏掉。遇到截断不会把未完成的原文尾部直接写出。配置缺失、非 UTF-8、空值或超出限制会在启动命令前失败。

最多指定 32 个不同变量，每个值最多 4096 UTF-8 字节。较长值优先匹配。原始参数和环境仍交给子进程；仅保存的参数被替换。日志记录会列出变量名，不保存这些配置值。

这不是自动密钥识别、加密或完整防泄露系统。编码后的值、不同形式的值、路径元数据、子进程自行创建的文件、远程服务和系统进程列表不受保护。没有明确配置时仍保存原始日志。配置后保存的是处理后的日志，不再声称它与原始输出逐字节一致；报告仍区分是否完整捕获与是否启用脱敏。

## 测试和可复现对比

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
python -m unittest discover -s tests -p 'test_*.py' -v
python scripts/benchmark.py --binary target/debug/cpyref --repeats 5
```

Windows 的二进制名是 `cpyref.exe`。Python 回归测试允许通过 `CPYREF_BIN` 指定已构建的程序，所有文件和敏感值均为临时的合成数据。CI 在 Linux、macOS、Windows 运行上述测试，另检查 Rust 1.89 的最低版本兼容性。

基准脚本对比普通文件复制、一次复制一次粘贴、复用既有引用，逐次检查输出字节和记录耗时。它不调用模型，不将字节数换算成实测 Token，也不声称模型会采用该工具。真实 Token、模型调用次数和端到端 Agent 时间仍需相同任务、模型及工具环境下的对照实验。
