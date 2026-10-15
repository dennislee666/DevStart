<p align="center"><img src="assets/devstart-window-icon.png" width="88" alt="DevStart"></p>

<h1 align="center">DevStart</h1>
<p align="center">一个简洁的 Rust 原生开发项目启动器。<br>A native Rust launcher for local development projects.</p>

DevStart 把散落在不同项目中的启动命令集中到一个桌面窗口：按分组管理项目，为同一项目配置多个启动项，同时运行、停止、重启，并切换或平铺查看 Console。

**脚本仍属于原项目。** DevStart 引用项目自己的文件、命令和参数，不接管项目依赖，不在导入时运行脚本。开源版初次启动没有项目、启动项或个人分组，只有一个空的默认分组。

## 界面预览

下面是 **Facebook 虚构项目分组**的真实应用截图：Java API、后台管理、官网、手机 H5，每个项目分别提供“正式生产库”和“测试库”两个启动项。

![Facebook 分组：四个项目、八个启动项，展示配置端口与运行状态](docs/screenshots/facebook-projects.png)

多个启动项同时运行时，可以切换标签查看各自的输出，也可以使用平铺视图。项目信息区显示状态、端口、PID、启动时间和运行时长；Console 支持搜索、过滤、彩色输出和 Control + 点击打开链接。

![运行 Console：标签切换、端口状态与彩色模拟日志](docs/screenshots/facebook-console.png)

| 项目 | 正式生产库端口 | 测试库端口 | 示例目录 |
| --- | --- | --- | --- |
| Java API | 18080 | 18081 | `~/Projects/facebook/java-api` |
| 后台管理 | 13000 | 13001 | `~/Projects/facebook/admin` |
| 官网 | 14000 | 14001 | `~/Projects/facebook/website` |
| 手机 H5 | 18000 | 18001 | `~/Projects/facebook/mobile-h5` |

**演示说明：** Facebook 仅作为虚构工作区名称，与 Meta / Facebook 无关联。截图来自隔离工作空间；Java/npm 命令是显示示例，实际由本地模拟进程产生输出，不连接生产或测试数据库，也不部署、打包或上传。示例分组不会预装到应用中，首次启动仍为空数据。详见[截图与演示说明](docs/showcase.md)。

## 功能

- **项目与启动项**：分组、收藏、搜索、“修改信息”统一编辑显示名和简介，同一项目多个运行参数；独立状态列与启动前配置端口展示。
- **进程管理**：同时启动多个项目；启动前核实旧实例，先优雅退出，超时再强制关闭；生命周期事件输出到 Console；当前标签内重启，批量启动/停止/重启均有二次确认。
- **Console**：标签切换与平铺、关键词搜索/过滤、跟随输出、独立窗口、ANSI 颜色、自动换行与链接打开；运行状态、端口、启动日期及累计时长。
- **源文件编辑**：Rust、Java、Python、Shell、前端等语法高亮，行号、自动换行，冲突检测、原子保存及每个启动项最近 10 个版本回退。
- **外观设置**：深色/浅色/系统主题，界面与 Console 字体、字号、行距、密度，可折叠侧栏。
- **AI 与自动化**：本地 JSON-RPC API、CLI、MCP，配置管理、启停、日志查询和编辑/回退均可调用。

## 平台与依赖

当前正式验证平台是 **macOS**。使用 Rust / eframe / egui，无需为 DevStart 安装 Node.js 或 WebView。Linux 的图形与进程工具路径仍需适配验证；Windows 暂未实现进程组管理，不承诺可用。

- Rust **1.88+**，建议使用稳定版工具链。
- macOS Command Line Tools；外部进程/端口识别使用系统 `ps` 和 `lsof`。
- 被启动项目仍需自行准备它的 JDK、Node.js、Python、Rust 等依赖。

## 快速开始

```bash
git clone --branch open-source https://github.com/dennislee666/DevStart.git
cd DevStart
cargo run --locked
```

添加一个本地项目，读取它自己的启动脚本；复杂入口可在项目根目录放置 `.dev-start.json`。导入和刷新只读取，不执行。

```json
{
  "version": 1,
  "launches": [
    {
      "name": "前端开发",
      "file": "package.json",
      "entry": { "Package": { "manager": "npm", "name": "dev" } },
      "description": "运行项目自身的 dev 命令",
      "expected_port": 5173
    }
  ]
}
```

原项目的 `package.json` 需实际提供 `scripts.dev`，端口需与实际项目一致。更多 File / Package / Command / Java 定义见[启动清单](docs/project-launch-manifest.md)。仓库中的示例不会自动导入。

## macOS 打包

```bash
bash scripts/package-macos.sh
open dist/DevStart.app
```

生成的应用为本机架构、临时本地签名，未进行 Apple 公证。仓库不包含预置的个人配置或已打包应用。

## AI / API / MCP

先启动 GUI，或以 `cargo run --locked -- --serve` 启动同一个工作空间的无界面服务，再由另一个终端调用：

```bash
cargo run --locked -- --api api.describe
cargo run --locked -- --api projects.list
```

MCP 客户端使用已编译的 `target/release/dev-start --mcp` 通过 stdio 连接本地服务。所有配置都由后端校验与持久化。完整方法、参数、日志分页与编辑器协议见 [API 文档](docs/api.md) 和 [Schema](docs/api-schema.json)。

## 数据与隐私

开源版的默认数据目录是 `~/Library/Application Support/devstart`，与个人版的 `dev-start` 目录隔离；CLI、MCP 和 GUI 使用相同规则。可通过 `DEV_START_DATA_DIR` 显式指定另一个独立目录。

配置、日志、备份和编辑历史留在本机，不提交到仓库。源文件只在明确保存或回退时写入；备份目录为用户私有权限。第三方脚本的行为与权限由用户负责选择。首次导入前没有运行命令；带有构建、部署、上传等名称的自动扫描入口会被排除，显式定义的禁用入口保持禁用。

## 开发与验证

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
python3 scripts/verify-api.py
# 可选：已安装 JDK 时验证 Java 源文件保存、编译失败隔离及回退
python3 scripts/verify-editor-java.py
```

验证脚本只使用临时目录和示例进程，不依赖个人项目。依赖锁定在 `Cargo.lock`，当前默认使用 RsProxy 国内镜像；来源与覆盖方法见[依赖说明](docs/dependency-sources.md)。

## 文档与参与

- [使用说明](docs/usage.md)
- [架构](docs/architecture.md)
- [项目启动清单](docs/project-launch-manifest.md)
- [API 与 AI 接入](docs/api.md)
- [贡献指南](docs/contributing.md)
- [安全与数据说明](docs/security.md)
- [开源/个人版本隔离](docs/release-channels.md)
- [依赖与第三方声明](docs/third-party-notices.md)

欢迎通过 Issues 和 Pull Requests 反馈问题或贡献代码。提交日志、截图和示例前，请移除本地路径、账号、令牌和业务数据。

## 许可证

项目代码使用 [MIT License](LICENSE)。第三方依赖、字体和图标保留其各自许可，见[第三方声明](docs/third-party-notices.md)。
