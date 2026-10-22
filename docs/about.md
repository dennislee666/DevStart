# DevStart 1.0 — 项目介绍 / About

## 中文

**适合 AI Coding 时代的多项目启动工作台。**

**项目多也不乱。** AI Coding 让一个人同时推进多个项目变得常见。DevStart 将找项目、启动、停止、重启和看日志集中在一个 Rust 原生桌面窗口，减少在目录和终端间反复切换。

用分组、搜索、收藏和运行中筛选找到项目，通过标签或平铺 Console 查看输出。当前项目需要本地 API 等相关服务时，找到对应项目、点击启动，再切回当前项目继续开发。相关项目与启动顺序由用户选择；DevStart 不自动解析跨项目依赖或提供复杂流程编排。

适合独立开发者在多个 AI Coding 项目、原型和前后端联调任务间高频切换。现有能力还包括端口与状态展示、启动文件编辑，以及供 AI 工具调用的本地 API、CLI 和 MCP。

### 当前能力

- **自动扫描项目**：选择目录后识别已有项目、启动项与可确定端口，勾选导入并自动分组、去重；不执行扫描到的代码，不向项目写管理文件。

- **项目管理**：分组、搜索、收藏和运行中筛选；同一项目可保留多个启动项及不同参数，显示名与简介可编辑。
- **运行与观察**：独立启动、停止、重启；状态、端口、PID 和运行时长；标签、平铺及独立 Console，支持日志搜索过滤、ANSI 颜色和链接打开。批量控制仅作用于已打开的 Console 标签，并需确认。
- **启动文件编辑**：语法高亮、冲突检测、原子保存及最近 10 个源文件版本回退；继续使用项目自身的文件和命令。
- **界面与接入**：中英文界面、主题与字体设置，以及本地 JSON-RPC API、CLI 和 MCP，供开发者和 AI 工具调用已有能力。

- **基础 Git 工作区**：在已登记项目间切换，选择已有本地分支和远程地址，查看变更、提交、推送及拉取更新并合并；浏览提交历史、文件新增/删除行数和多标签 Diff。详见 [Git 工作区](git-workspace.md)。
- **项目交互终端**：在项目目录使用持续 shell 会话，支持命令历史、实时彩色输出和 Ctrl+C；Git 页与运行页共享项目终端，运行页可切换启动日志与终端。

当前正式验证平台为 macOS 12 及以上，提供 Intel / Apple Silicon Universal 安装包。DevStart 本身无需 Node.js 或 WebView；项目所需的运行时与依赖仍需自行准备。开源版首次启动为空工作区，仅有默认分组；配置、日志与编辑历史保存在本机，不随仓库分发。

下载、安装、启动和技术说明见[中文 README](../README.md)。

**未来计划：** 基础 Git 工作区已提供跨项目的日常仓库操作；用户身份与凭据沿用现有 Git 配置，选择远程地址不等于切换账户。计划增加**多账户身份与凭据管理**，具体范围与时间待定。后续改进继续围绕多项目高频切换，保持操作简单。

## English

**A multi-project launch workbench for the AI Coding era.**

**More projects. Less friction.** AI Coding makes it common for one person to work across several projects. DevStart brings finding projects, starting, stopping, restarting, and reading logs into one native Rust desktop window, reducing repeated trips between directories and terminals.

Use groups, search, favorites, and the running filter to find a project. Switch Console tabs or tile them to inspect output. When your current project needs a local API or another related service, find that project, click to start it, then switch back and keep working. You choose related projects and their launch order; DevStart does not automatically resolve cross-project dependencies or provide complex workflow orchestration.

Built for individual developers switching frequently between AI Coding projects, prototypes, and frontend/API work. Current features also include port and status information, launch-file editing, and a local API, CLI, and MCP for AI tools.

### Current capabilities

- **Auto-scan projects**: choose a directory, review detected projects, launch entries and known ports, then import selected entries with grouping and deduplication. Scanning never executes code or writes management files into projects.

- **Project management**: groups, search, favorites, and a running filter; multiple launch entries and argument sets per project, with editable display names and descriptions.
- **Running and observing**: individual start/stop/restart controls; status, ports, PID, and elapsed time; tabbed, tiled, and separate Consoles with log search/filtering, ANSI colors, and link opening. Batch controls apply only to open Console tabs and require confirmation.
- **Launch-file editing**: syntax highlighting, conflict detection, atomic saves, and rollback through the latest 10 source-file versions, while continuing to use each project's own files and commands.
- **Interface and access**: English and Simplified Chinese, theme and font settings, and a local JSON-RPC API, CLI, and MCP for developers and AI tools to use existing capabilities.

- **Basic Git workspace**: switch between registered projects, select existing local branches and remote addresses, inspect changes, commit, push, and fetch updates with a merge; browse commit history, per-file additions/deletions, and multi-tab diffs. See the [Git workspace guide](git-workspace.md).
- **Interactive project terminals**: persistent shell sessions in project directories, with command history, live colored output, and Ctrl+C. Git and run views share the project terminal; the run view lets you switch between launch logs and the terminal.

The currently validated release platform is macOS 12 or later, with Intel / Apple Silicon Universal packages. DevStart itself needs no Node.js or WebView; each project still requires its own runtime and dependencies. On first run, the open-source edition creates a local demo group with four fictional projects; loopback-only mock launch entries require an explicit click. Existing catalogs are preserved. Configuration, logs, and edit history remain on the local computer and are not distributed with the repository.

See the [English README](../README_EN.md) for downloads, installation, launching, and technical details.

**Planned:** The basic Git workspace already supports everyday repository operations across projects. Identities and credentials follow your existing Git configuration; selecting a remote address does not switch accounts. **Multi-account identity and credential management** is planned, with scope and timing to be determined. Future improvements will continue to prioritize simple, frequent multi-project operations.

## 联系 / Contact

- 作者：dennis lee
- GitHub：https://github.com/dennislee666/DevStart
- 反馈与捐赠联系：danislee@gmail.com

## 交互与架构

侧栏顶部依次显示独立的收展按钮、App 图标、DevStart。收起时保留收展按钮。

macOS 菜单栏在 DevStart 右侧提供“关于”（英语 About US），其中的“关于 DevStart”打开居中的介绍框。原有应用菜单的 About 入口指向同一介绍框。设置中也提供入口。

原生菜单仅发出打开请求，由主界面显示本地化模态框；不会阻塞后台服务与本地 API。版本统一来自 Cargo.toml，安装包及 API 使用 1.0.0，界面显示 1.0。介绍框列出作者、仓库和联系邮箱；链接须点击才打开。

## 文案维护

中英文介绍位于 `src/ui_about.rs` 和 `src/locales/en.json`，与中英文 README 的定位保持一致。修改介绍不代表重新构建或发布安装包；构建与验证步骤见 [README](../README.md#开发与验证)。
