# 独立交互终端

侧栏“终端”位于运行中上方。主窗口每个标签使用独立 PTY 与当前 SHELL 的登录交互环境，初始目录为 HOME，不沿用项目终端的禁用历史和行编辑配置。命令由 shell 正常解释，支持补全、历史、Ctrl+C 和终端应用。

新标签插在最前，切换页面保留各标签进程并回到最后选中的标签。关闭应用会结束终端进程；重开终端页恢复标签数量、选中位置与主题，重新启动 shell，不宣称恢复上次仍运行的命令。关闭标签会关闭它的 PTY，用户应先结束需要保留的前台任务。

默认 mac 方案为黑底绿字，Linux 为黑底常规彩色方案。两种方案均按程序输出的 ANSI 指令显示 16/256/真彩色，不按内容猜测状态颜色，不强行给没有颜色指令的普通文本添加语义颜色。PTY 局部设置 TERM=xterm-256color、COLORTERM=truecolor、CLICOLOR=1，用户 shell 配置及工具自身的颜色选项仍然有效。配色在工作空间 desktop-terminal-state.json 保存。

终端引擎复用 egui_term 0.1.0 / Alacritty 0.25。从国内 rsproxy 镜像下载，vendor/egui_term 保留 MIT 许可；本地修改仅为 egui 0.33 的字体绘制接口适配与 PTY 局部环境。未回退国外依赖下载源。无法承诺复制 macOS Terminal.app 的所有偏好设置和系统集成功能。

快捷键：终端页支持 ⌘T 新建标签、⌘W 关闭当前标签；终端内保留 Ctrl+C 中断、Tab 补全、方向键历史与 ⌘C/⌘V 复制粘贴。新增 shell.list/open/read/input/select/close/theme API 和独立 MCP 工具，精确契约见 api.md。实际验证包含两套主题的 ANSI 多色输出、HOME 初始目录、新标签置顶、切页保留选中标签、独立环境变量；不声称验证了 macOS Terminal 的所有系统集成。
