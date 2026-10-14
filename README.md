# CpyRef

**Copy once. Paste exactly.**

A planned cross-platform copy-and-paste tool for AI agents, written in Rust.

为 Agent 保存原文、按编号准确粘贴的跨平台工具。

## 项目状态

当前处于设计阶段：本仓库只有项目说明，尚未实现 CLI，也尚未发布 Rust 包。下方命令是拟议接口，不是可运行的功能。

## 为什么需要 CpyRef

Agent 处理超长链接、文件片段、代码或工具输出时，不应为了复制内容而重新逐字符生成原文。CpyRef 计划保存原文并返回短编号，让 Agent 使用编号准确粘贴，减少重复输出和转写错误。

`cpy` 表示 copy，`ref` 表示 reference：保存一次原文，通过引用复用。用途不局限于系统剪贴板。

## 统一命名

- 项目展示名：**CpyRef**
- 命令行工具：`cpyref`
- GitHub 仓库：`LIghtJUNction/cpyref`
- 计划使用的 Rust 包名：`cpyref`（尚未发布或保留）

## 拟议命令

以下接口尚未实现，具体语义可能在开发中调整。

```sh
# 保存文件中的指定行，只返回短编号
cpyref copy src/main.rs --lines 20:40

# 将指定编号的原文写入新文件
cpyref paste r7k2 --output snippet.rs

# 查看来源、大小等信息，不展开全文
cpyref info r7k2
```

## 设计方向

- 保存原文，通过短编号引用，而不是要求模型重新生成内容。
- 支持文件片段、代码、链接和工具输出等使用场景。
- 使用 Rust 开发，面向跨平台命令行使用。
- 信息查询默认只返回元数据，避免无意展开大量原文。
