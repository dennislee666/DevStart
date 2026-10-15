# 依赖来源与工具链

`Cargo.lock` 固定依赖版本。项目的 `.cargo/config.toml` 默认使用 RsProxy 国内 sparse 镜像 `https://rsproxy.cn/index/`。本项目验证时该来源可用，没有通过不明来源分发依赖。

如果你的网络无法使用该镜像，可在自己的 checkout 中调整或移除 `.cargo/config.toml`，恢复 Cargo 官方 crates.io 来源，并记录构建所用来源。不要修改 lockfile 来绕过校验。

主要组件：eframe/egui 0.33、egui_extras + Syntect、egui-phosphor、rfd、serde/serde_json、libc、SHA-256、unicode-segmentation，以及 macOS objc2 API。编辑器不依赖浏览器、Node.js 或外部语言服务器。字体优先复用系统字体，图标使用 Phosphor 字库与项目自己的图形资源。

第三方许可声明见 third-party-notices.md；依赖版本更新后需同步审核并重新运行测试。
