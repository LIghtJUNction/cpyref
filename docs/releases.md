# 安装与发布

## 安装

Cargo：

```sh
cargo install cpyref --locked
```

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

每次 Release 都附带 `SHA256SUMS`，可在下载后核对文件摘要。压缩包也包含 MIT 许可证与使用文档。

## 发布新版本

1. 修改 `Cargo.toml` 的版本号，并用 `cargo generate-lockfile` 更新 `Cargo.lock` 中的本包版本。
2. 本地运行格式检查、测试、Clippy 和 `cargo publish --locked --dry-run`，提交并推送源码。
3. 创建与包版本一致的 tag，例如版本 `0.1.1` 对应 `v0.1.1`，并推送：

```sh
git tag -a v0.1.1 -m 'CpyRef 0.1.1'
git push origin v0.1.1
```

推送 `v*` tag 会触发 `.github/workflows/release.yml`。工作流核对 tag 与 Cargo 版本，分别在六种原生运行环境中测试和构建，生成压缩包与 Linux 发行版包，并在全部构建成功后发布 GitHub Release 及摘要文件。

网络等临时故障可通过 GitHub Actions 的手动入口选择原 tag 重试。如果需要修改源码或工作流，更新版本并创建新 tag。已公开 Release 的文件不会被自动覆盖。

crates.io 和 AUR 独立发布，不会使用 GitHub Actions 中的仓库令牌自动上传：

```sh
cargo publish --locked
```

AUR 打包文件位于 `packaging/aur/`。更新版本与源码 SHA-256，在 Arch Linux 上运行 `makepkg` 验证后，用 `makepkg --printsrcinfo` 更新 `.SRCINFO`，再提交到 AUR 包仓库的 `master`。
