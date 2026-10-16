# DevStart 1.0 — About

一个软件，启动您电脑上的所有项目。

AI 与 Vibe Coding 正在加快软件开发，也让开发者同时维护更多项目。DevStart 将项目分组、启动控制、端口与实时日志监控集中在一个窗口，并提供启动代码编辑和本地 API，方便开发者与 AI 工具协作。

- 作者：dennis lee
- GitHub：https://github.com/dennislee666/DevStart
- 反馈与捐赠联系：danislee@gmail.com

## 交互与架构

侧栏顶部依次显示独立的收展按钮、App 图标、DevStart。收起时保留收展按钮。

macOS 菜单栏在 DevStart 右侧提供“关于”（英语 About US），其中的“关于 DevStart”打开居中的介绍框。原有应用菜单的 About 入口指向同一介绍框。设置中也提供入口。

原生菜单仅发出打开请求，由主界面显示本地化模态框；不会阻塞后台服务与本地 API。版本统一来自 Cargo.toml，安装包及 API 使用 1.0.0，界面显示 1.0。介绍框列出作者、仓库和联系邮箱；链接须点击才打开。

## 验证

已验证中英文介绍框、菜单随语言切换、居中布局与独立侧栏按钮。Universal macOS 包包含 arm64 和 x86_64，最低系统要求保持 macOS 12。语言验证完成后恢复用户原有设置，3 个服务进程保持运行。
