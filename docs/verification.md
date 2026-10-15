# 验证记录

2026-10-15，macOS 原生开发环境：

- `cargo fmt --check` 通过。
- `cargo clippy --all-targets --locked -- -D warnings` 通过。
- `cargo test --locked`：105 项通过。
- `cargo build --release --locked` 通过，版本标识为 `open-source`。
- `scripts/verify-api.py`：隔离工作空间 API、CLI、MCP、配置、编辑器、运行、日志和批量计划验证通过。
- `scripts/verify-editor-java.py`：隔离 Java 源文件保存、编译失败保持原服务、实际输出更新与最近十版本回退通过。
- 公开版独立 HOME 默认启动：无项目、无运行，只有空默认分组，不自动导入演示数据。
- Facebook 两张原生截图使用虚构项目、路径、端口和合成日志；发布文件不含个人工作空间与截图文字/EXIF 元数据。

本记录不代表 Linux/Windows 已通过平台验证，也不代表任意第三方项目脚本已经过验证。
