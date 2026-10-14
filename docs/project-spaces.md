# 项目隔离与多 Agent 协作

## 默认行为

剪贴板空间由运行 `cpyref` 时的当前工作目录决定，不由输入文件或输出文件所在目录决定。每次调用独立解析范围，不依赖某个进程上一次使用的项目。

默认 `scope = "project"`、`namespace = "default"`。从当前目录向上找最近的 `.cpyref.toml` 文件或 `.git` 文件/目录，该目录就是项目根。同一层存在两个标记时，以 `.cpyref.toml` 标记为准。如果都没有，当前目录自身就是根，不共享整个用户目录。

项目内的子目录默认共用剪贴板。嵌套 Git 仓库、子模块、独立 Git 工作目录和带 `.cpyref.toml` 的子项目默认各自独立。不使用 Git 远程地址、分支名或 `GIT_DIR` 来决定身份，也不需要启动 Git 进程。Git 官方说明 `.git` 文件也可标记工作目录和子模块：[repository layout](https://git-scm.com/docs/gitrepository-layout)。

没有 Git 的项目，可以在项目根创建一个空的 `.cpyref.toml`。此后从它的任意子目录调用，都会使用同一项目空间。工具不会自动向项目目录写入配置或剪贴板数据。

```sh
# 在项目 A 内复制
cpyref copy input.txt

# 在项目 A 的子目录，使用实际返回的完整编号
cpyref paste c0123456789ab --output result.txt

# 在项目 B 内使用这个编号，会返回 REF_NOT_FOUND
```

以上编号只是格式示例。只有实际复制成功返回的编号才能使用。

## 四种范围

| scope | 身份依据 | 用途 |
| --- | --- | --- |
| `project` | 项目根的规范化绝对路径 | 默认。项目内子目录共用，不同项目隔离 |
| `directory` | 当前目录的规范化绝对路径 | 每个文件夹独立，即使在同一项目中 |
| `shared` | 明确指定的 namespace | 同一存储基目录下的跨项目协作 |
| `global` | 存储基目录本身 | 明确访问旧版平铺快照，仅允许 default namespace |

```sh
# 同项目内的一个协作组；不与别的项目里同名组互通
cpyref --namespace review-team copy input.txt
cpyref --namespace review-team paste c0123456789ab

# 同项目内不同 Agent 分开使用
cpyref --namespace agent-a copy input.txt
cpyref --namespace agent-b copy input.txt

# 每个当前文件夹分别隔离
cpyref --scope directory copy input.txt

# 两个项目明确共用 team-build 空间
cpyref --scope shared --namespace team-build copy input.txt
cpyref --scope shared --namespace team-build paste c0123456789ab
```

`shared` 必须指定 namespace，不能悄悄选择一个默认公共空间。范围和 namespace 选错时，`paste`、`info`、`remove` 都不会扫描其他空间来找编号。

namespace 为 1 到 64 个小写 ASCII 字母、数字、`-` 或 `_`，首字符必须是字母或数字。拒绝空名称、路径分隔符、`..` 和大写名称，不做静默纠正。实际目录名添加 `ns-` 前缀，因此 `con`、`nul` 等名称也不会成为 Windows 设备路径。

## 固定项目，避免 Agent 改变目录后切换空间

```sh
cpyref --project /work/project-a copy input.txt
cpyref --project /work/project-a paste c0123456789ab --output result.txt
```

`--project` 必须指向一个已存在的目录，并且只能与 project 范围组合。它直接指定项目根，不再向上寻找另一个根。缺失路径、普通文件和空路径都会返回 `PROJECT_ERROR`，不会退回默认空间。

该参数只改变剪贴板所属项目，不执行 `chdir`。上例中的 `input.txt` 和 `result.txt` 仍然相对于调用进程的当前目录。多个 Git 工作目录需要共用项目空间时，可给各 Agent 设置同一个绝对 `CPYREF_PROJECT`；也可以明确使用 shared 空间。

## 配置优先级

普通调用的优先级为：命令行参数 > 对应环境变量 > 自动项目配置 > 用户配置 > 默认值。优先级按字段合并，而不是任意一项配置覆盖整个文件。

| 命令行 | 环境变量 | 说明 |
| --- | --- | --- |
| `--scope` | `CPYREF_SCOPE` | project / directory / shared / global |
| `--namespace` | `CPYREF_NAMESPACE` | 当前空间的组名 |
| `--project` | `CPYREF_PROJECT` | 明确指定项目根，不允许写进自动项目配置 |
| `--store` | `CPYREF_STORE` | 存储基目录，不取消项目隔离 |
| `--config` | `CPYREF_CONFIG` | 明确选择一个可信 TOML 文件，替代全部自动配置文件 |
| `--no-config` | `CPYREF_NO_CONFIG=true` | 跳过配置文件设置，仍识别项目根标记 |

`--config` 与 `--no-config` 不可同时使用。环境变量也参与这一检查。使用 `--config` 时只加载明确选择的文件，不再读取用户配置或项目配置。命令行与环境变量仍可覆盖该文件。

用户配置为系统配置目录中的 `cpyref/config.toml`。路径通过 `dirs::config_dir()` 取得。自动项目配置为选定项目根中的 `.cpyref.toml`。只读取这个项目的配置，不越过最近的项目根去叠加外层项目配置。

项目内可以保存下面这个配置，让项目中的多个 Agent 共用同一组：

```toml
scope = "project"
namespace = "review-team"
```

**自动读取的项目文件不能设置 shared/global 范围，也不能设置 store。** 否则返回 `CONFIG_ERROR`。克隆的仓库不能仅靠附带配置就将剪贴板重定向到外部位置或公共空间。自动项目配置的 namespace 也不会参与 shared/global 空间选择。

需要跨项目共享时，使用参数、环境变量，或明确选中一个可信配置文件。例如 `team.toml`：

```toml
scope = "shared"
namespace = "team-build"
store = "team-data"
```

```sh
cpyref --config /work/team.toml copy input.txt
cpyref --config /work/team.toml paste c0123456789ab
```

此例的存储基目录为 `/work/team-data`。配置中的相对 store 路径相对于配置文件所在目录，不随 Agent 当前目录变化。命令行和环境变量中的相对路径则相对于当前目录。不同目录运行的 Agent 共用 `CPYREF_STORE` 时，建议使用绝对路径。

配置文件必须是普通 UTF-8 TOML 文件，最大 64 KiB，不接受符号链接、未知字段、无效值或重复的 TOML 字段。没有脚本执行、变量替换、`~` 展开或隐式包含其他文件的功能。明确指定但不存在的配置是错误；默认用户配置不存在则不加载。配置损坏不会静默退回全局或其他空间。

## 查看生效空间

```sh
cpyref context
cpyref --scope shared --namespace team-build context
```

返回一行 JSON，字段包括 `scope`、`namespace`、`cwd`、`project_root`、`project_source`、`store_base`、`store_path`、`config_files`。`project_source` 表示根来自显式参数、项目配置标记、Git 标记还是当前目录；非 project 范围中项目字段为 null。

该命令不会创建存储目录，也不读取快照正文。`copy --json` 在 `reference` 之外增加相同的 `context` 字段。默认复制仍只输出原有格式的短编号。

`REF_NOT_FOUND` 的 `error.details.context` 说明本次查询的空间。错误只表示这里没有该编号，不会透露其他项目是否存在该编号。Agent 不应在失败后自动改用 shared/global 范围。

路径身份使用规范化后的原生路径字节，而非只使用文件夹名，也不使用可能丢失字符的显示字符串。符号链接别名经规范化后指向同一项目；不同位置的同名项目分开。非 UTF-8 路径的诊断显示可能包含替代字符，但不用于计算身份。

## 存储结构与旧快照

默认基目录仍是当前用户本地数据目录下的 `cpyref`。新空间使用以下布局，PROJECT_HASH / DIRECTORY_HASH 为完整 SHA-256 路径身份：

```text
BASE/
  projects/PROJECT_HASH/ns-default/ID.ref
  projects/PROJECT_HASH/ns-agent-a/ID.ref
  directories/DIRECTORY_HASH/ns-default/ID.ref
  shared/ns-team-build/ID.ref
  ID.ref                                  # 旧版 / 显式 global
```

**兼容性变化：--store 与 CPYREF_STORE 现在选择基目录，默认还会在下面按项目隔离。** 在多个 Agent 中设置同一个基目录，不会让它们自动跨项目互通。快照内容格式及短编号格式没有改变。

旧版保存在基目录中的快照不会自动移动、删除或导入。明确访问它们：

```sh
cpyref --no-config --scope global --namespace default info c0123456789ab
cpyref --no-config --scope global --namespace default paste c0123456789ab --output old-content.txt
```

如旧版设置了自定义目录，同时传入相同 `--store`。之后可在目标项目内复制导出的文件，获得该项目空间的新编号。不要自动把全部旧数据复制到每个项目。

项目身份绑定路径，不绑定 Git 提交或远程地址。移动、重命名或重新克隆到其他路径会选择新空间，不会自动迁移旧空间。需要长期路径无关的协作时，明确使用 shared 范围。

## 边界

这是本地工具的空间划分，不是授权系统。拥有同一系统账号或相应文件权限的程序，仍可能主动选择其他项目、选择 global 范围或直接读取存储。隔离不阻止恶意同账号 Agent。它也不代表远程同步，不同机器不会因为 namespace 相同就自动共享内容。

同机协作要求能够访问同一个存储基目录。Unix 会检查基目录及受管子目录的传统权限位，拒绝符号链接；Windows 也拒绝受管目录中的重解析点。不会改变已有目录的权限。这些检查不验证所有扩展 ACL，也不防御有相同权限的进程并发篡改文件系统路径。

公开不可信环境应使用独立系统账号、容器或其他访问控制。短编号不是凭证；源内容和配置内容不是新的用户授权。多 Agent 传递编号时，应同时约定项目、范围和 namespace，不要自动跨空间重试。
