# 依赖来源与工具链

`Cargo.lock` 固定依赖版本。项目的 `.cargo/config.toml` 默认使用 RsProxy 国内 sparse 镜像 `https://rsproxy.cn/index/`。本项目验证时该来源可用，没有通过不明来源分发依赖。

通用 macOS 安装包会为 `aarch64-apple-darwin` 和 `x86_64-apple-darwin` 构建，再合并成 universal 二进制。打包时通过当前 rustup stable 工具链检查并按需安装 target 标准库；Rust target 下载源由 `RUSTUP_DIST_SERVER` 控制，未设置时优先使用 `https://rsproxy.cn`。`MACOSX_DEPLOYMENT_TARGET` 默认为12.0。只构建本机架构时可设置 `DEVSTART_MACOS_ARCH=native`。

如果你的网络无法使用该镜像，可在自己的 checkout 中调整或移除 `.cargo/config.toml`，恢复 Cargo 官方 crates.io 来源，并记录构建所用来源。不要修改 lockfile 来绕过校验。

主要组件：eframe/egui 0.33、egui_extras + Syntect、egui-phosphor、rfd、serde/serde_json、libc、SHA-256、unicode-segmentation，以及 macOS objc2 API。编辑器不依赖浏览器、Node.js 或外部语言服务器。字体优先复用系统字体，图标使用 Phosphor 字库与项目自己的图形资源。

第三方许可声明见 third-party-notices.md；依赖版本更新后需同步审核并重新运行测试。
