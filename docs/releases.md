# 安装与发布

## 安装

Cargo：

```sh
cargo install cpyref --locked
```

程序名是 `cpyref`。用户说“用 cpy 交接”时，Agent 应调用 `cpyref`；想在 POSIX 交互终端使用简称，可选执行 `alias cpy=cpyref`，并按需自行加入所用 shell 的配置。脚本仍调用 `cpyref`，安装过程不修改 shell 配置，也不安装全局 `cpy` 命令。

一步交接、有限读取和 AgentGC 联合示例要求 CpyRef 0.4.0+。升级后用 `cpyref --version` 确认 PATH 中实际运行的版本；各功能的最低版本见 [README](../README.md)。

Arch Linux 的 AUR 源码包：

```sh
yay -S cpyref
```

[GitHub Releases](https://github.com/LIghtJUNction/cpyref/releases) 提供以下二进制包，每个平台均有 x64 和 ARM64 版本：

| 系统 | 格式 | 安装方式 |
| --- | --- | --- |
| Debian / Ubuntu | `.deb` | `sudo apt install ./cpyref-*.deb`，选择与机器架构对应的单个文件 |
| Fedora / RHEL 系 | `.rpm` | `sudo dnf install ./cpyref*.rpm`，选择与机器架构对应的单个文件 |
| Arch Linux | `.pkg.tar.zst` | `sudo pacman -U ./cpyref*.pkg.tar.zst`，选择与机器架构对应的单个文件 |
| 通用 Linux | `.tar.gz` | 解压后把 `cpyref` 放入 PATH 中的目录 |
| Windows | `.zip` | 解压后把 `cpyref.exe` 所在目录加入 PATH |
| macOS | `.tar.gz` | 解压后把 `cpyref` 放入 PATH 中的目录 |

Linux 二进制使用 musl 静态链接。Windows 使用 MSVC；macOS 分 Intel 与 Apple Silicon 两种构建。Windows/macOS 文件没有 Authenticode 或 Apple Developer ID 签名。

每次 Release 都附带 `SHA256SUMS`，可在下载后核对文件摘要。压缩包也包含构建该版本时适用的许可证、使用文档和 `skills/cpyref` 配套 Agent 技能。Linux 发行版包将技能放在 `/usr/share/doc/cpyref/skills/cpyref`。

## 发布新版本

1. 修改 `Cargo.toml` 的版本号，并用 `cargo generate-lockfile` 更新 `Cargo.lock` 中的本包版本。
2. 本地运行格式检查、测试、Clippy 和 `cargo publish --locked --dry-run`，提交并推送源码。
3. 创建与包版本一致的 tag，例如版本 `0.1.2` 对应 `v0.1.2`，并推送：

```sh
git tag -a v0.1.2 -m 'CpyRef 0.1.2'
git push origin v0.1.2
```

推送 `v*` tag 会触发 `.github/workflows/release.yml`。工作流核对 tag 与 Cargo 版本，分别在六种原生运行环境中测试和构建，生成压缩包与 Linux 发行版包，并在全部构建成功后发布 GitHub Release 及摘要文件。

网络等临时故障可通过 GitHub Actions 的手动入口选择原 tag 重试。如果需要修改源码或工作流，更新版本并创建新 tag。已公开 Release 的文件不会被自动覆盖。

crates.io 和 AUR 独立发布，不会使用 GitHub Actions 中的仓库令牌自动上传：

```sh
cargo publish --locked
```

AUR 打包文件位于 `packaging/aur/`。更新版本与源码 SHA-256，在 Arch Linux 上运行 `makepkg` 验证后，用 `makepkg --printsrcinfo` 更新 `.SRCINFO`，再提交到 AUR 包仓库的 `master`。

许可证：已发布的 0.2.2、0.3.0 和 0.3.1 采用 AUL 1.0-draft.1，当前源码延续该许可证；0.1.x 历史发行版仍是 MIT，原有 MIT 授权不变。发布时核对 crate、AUR 元数据与安装包中的 LICENSE，并保留 LICENSE.MIT；不要把旧 MIT 构建重新标记为 AUL。
